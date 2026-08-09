"""Unit tests for the benchmark scoring math (no OpenCV required)."""

import unittest
from pathlib import Path

from scene_vault_ai.eval import (
    calibrate,
    cosine_similarity,
    distribution_summary,
    evaluate_with_params,
    evaluate_game,
    load_manifest,
    manifest_dataset,
    percentile,
    rank_characters,
    suggest,
    topk_mean,
)


class CosineSimilarityTests(unittest.TestCase):
    def test_identical_vectors(self) -> None:
        self.assertAlmostEqual(cosine_similarity([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), 1.0)

    def test_orthogonal_vectors(self) -> None:
        self.assertAlmostEqual(cosine_similarity([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), 0.0)

    def test_dimension_mismatch_returns_zero(self) -> None:
        self.assertEqual(cosine_similarity([1.0, 0.0], [1.0, 0.0, 0.0]), 0.0)

    def test_empty_vector_returns_zero(self) -> None:
        self.assertEqual(cosine_similarity([], [1.0, 0.0]), 0.0)

    def test_zero_vector_returns_zero(self) -> None:
        self.assertEqual(cosine_similarity([0.0, 0.0], [1.0, 0.0]), 0.0)


class TopkMeanTests(unittest.TestCase):
    def test_top_two_mean(self) -> None:
        self.assertAlmostEqual(topk_mean([1.0, 3.0, 2.0], 2), 2.5)

    def test_k_larger_than_list_means_everything(self) -> None:
        self.assertAlmostEqual(topk_mean([1.0, 3.0], 5), 2.0)

    def test_empty_list_returns_zero(self) -> None:
        self.assertEqual(topk_mean([], 3), 0.0)


class RankTests(unittest.TestCase):
    BANK = {
        "Ava": [[1.0, 0.0, 0.0]],
        "Bella": [[0.0, 1.0, 0.0]],
    }

    def test_orders_characters_by_score(self) -> None:
        ranked = rank_characters([1.0, 0.0, 0.0], self.BANK, topk=3)
        self.assertEqual(ranked[0].character, "Ava")
        self.assertGreater(ranked[0].score, ranked[1].score)
        self.assertAlmostEqual(ranked[0].score, 1.0)

    def test_excludes_characters_without_bank_samples(self) -> None:
        bank = {
            "Ava": [[1.0, 0.0, 0.0]],
            "KoKo": [],  # all its images were filtered out
        }
        ranked = rank_characters([1.0, 0.0, 0.0], bank, topk=1)
        self.assertEqual(
            [entry.character for entry in ranked],
            ["Ava"],
            "characters with no usable sample cannot be ranked",
        )

    def test_topk_choice_changes_the_score(self) -> None:
        multi_sample = {
            "Ava": [[1.0, 0.0, 0.0], [0.9, 0.1, 0.0]],
            "Bella": [[0.0, 1.0, 0.0]],
        }
        # topk=1 is the "most similar prototype" rule.
        ranked_max = rank_characters([1.0, 0.0, 0.0], multi_sample, topk=1)
        self.assertAlmostEqual(ranked_max[0].score, 1.0)
        # topk=3 averages the character's top samples.
        ranked_mean = rank_characters([1.0, 0.0, 0.0], multi_sample, topk=3)
        second = 0.9 / ((0.9**2 + 0.1**2) ** 0.5)
        self.assertAlmostEqual(ranked_mean[0].score, (1.0 + second) / 2, places=3)


class SuggestTests(unittest.TestCase):
    BANK = {
        "Ava": [[1.0, 0.0, 0.0]],
        "Bella": [[0.0, 1.0, 0.0]],
    }

    def test_suggests_when_threshold_and_margin_pass(self) -> None:
        self.assertEqual(
            suggest([1.0, 0.0, 0.0], self.BANK, topk=1, threshold=0.6, margin=0.1),
            "Ava",
        )

    def test_below_threshold_means_no_suggestion(self) -> None:
        self.assertIsNone(
            suggest([0.5, 0.5, 0.0], self.BANK, topk=1, threshold=0.8, margin=0.0)
        )

    def test_small_margin_means_no_suggestion(self) -> None:
        # The top score clears the threshold but the runner-up is too close:
        # Scene Vault prefers no suggestion over a confident wrong one.
        self.assertIsNone(
            suggest([0.5, 0.5, 0.0], self.BANK, topk=1, threshold=0.45, margin=0.1)
        )

    def test_margin_zero_allows_close_calls(self) -> None:
        self.assertEqual(
            suggest([0.5, 0.5, 0.0], self.BANK, topk=1, threshold=0.45, margin=0.0),
            "Ava",
        )


class EvaluateGameTests(unittest.TestCase):
    def test_counts_hits_grid_and_distributions(self) -> None:
        bank = {
            "Ava": [[1.0, 0.0, 0.0]],
            "Bella": [[0.0, 1.0, 0.0]],
        }
        queries = [
            ("Ava", [1.0, 0.0, 0.0]),
            ("Bella", [0.0, 1.0, 0.0]),
            ("Bella", [0.5, 0.5, 0.0]),  # tie with Ava: never a suggestion
        ]
        result = evaluate_game(
            name="demo",
            bank=bank,
            queries=queries,
            topk_values=(1,),
            thresholds=(0.6,),
            margins=(0.1,),
        )
        self.assertEqual(result.queries, 3)
        self.assertEqual(result.top1_hits, 2)
        self.assertEqual(result.top3_hits, 3)
        self.assertEqual(result.rank_distribution, (2, 1, 0, 0))
        self.assertEqual(len(result.same_scores), 3)
        self.assertEqual(len(result.cross_scores), 3)
        entry = result.grid[0]
        self.assertEqual(entry.suggestions, 2)
        self.assertEqual(entry.correct, 2)
        self.assertAlmostEqual(entry.precision, 1.0)
        self.assertAlmostEqual(entry.coverage, 2 / 3)
        self.assertAlmostEqual(entry.false_suggest_rate, 0.0)

    def test_same_scores_collect_true_character_best_match(self) -> None:
        bank = {
            "Ava": [[1.0, 0.0, 0.0]],
            "Bella": [[0.0, 1.0, 0.0]],
        }
        result = evaluate_game(
            name="demo",
            bank=bank,
            queries=[("Ava", [1.0, 0.0, 0.0])],
            topk_values=(1,),
            thresholds=(0.5,),
            margins=(0.0,),
        )
        self.assertAlmostEqual(result.same_scores[0], 1.0)
        self.assertAlmostEqual(result.cross_scores[0], 0.0)

    def test_character_without_bank_samples_does_not_crash(self) -> None:
        bank = {
            "Ava": [[1.0, 0.0, 0.0]],
            # Every image of this character failed face detection, so the
            # bank ends up empty; evaluation must not crash on it.
            "KoKo": [],
        }
        result = evaluate_game(
            name="demo",
            bank=bank,
            queries=[("KoKo", [1.0, 0.0, 0.0])],
            topk_values=(1,),
            thresholds=(0.5,),
            margins=(0.0,),
        )
        self.assertEqual(result.queries, 1)
        self.assertEqual(result.top1_hits, 0)
        self.assertEqual(len(result.cross_scores), 1)

    def test_headline_metrics_use_max_not_topk_mean(self) -> None:
        # A: one perfect match plus two orthogonal samples -> max = 1.0 but
        # top3 mean = 1/3. B: two near-perfect samples -> max ~0.999 but
        # top2 mean ~0.999. Production uses max, so A must win the headline
        # Top-1 even though a topk=3 aggregation would rank B first.
        bank = {
            "Ava": [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            "Bella": [
                [0.95, 0.05, 0.0],
                [0.95, -0.05, 0.0],
            ],
        }
        result = evaluate_game(
            name="demo",
            bank=bank,
            queries=[("Ava", [1.0, 0.0, 0.0])],
            topk_values=(1, 3),
            thresholds=(0.3,),
            margins=(0.0,),
        )
        self.assertEqual(result.top1_hits, 1)
        self.assertEqual(result.top3_hits, 1)
        self.assertEqual(result.rank_distribution, (1, 0, 0, 0))


class CalibrationTests(unittest.TestCase):
    def test_calibrate_picks_highest_coverage_at_min_precision(self) -> None:
        bank = {
            "Ava": [[1.0, 0.0, 0.0]],
            "Bella": [[0.0, 1.0, 0.0]],
        }
        queries = [
            ("Ava", [1.0, 0.0, 0.0]),
            ("Bella", [0.0, 1.0, 0.0]),
            ("Ava", [0.5, 0.5, 0.0]),  # tie: never suggested with margin
        ]
        point = calibrate(
            bank=bank,
            queries=queries,
            thresholds=(0.45, 0.5),
            margins=(0.0, 0.1),
            min_precision=1.0,
            min_suggestions=1,
        )
        # At 0.45/0.0 the ambiguous query gets suggested (wrong? it is a
        # tie, suggestion = Ava = correct) -> 3/3 precision 1.0; at
        # 0.5/0.1 only two queries are suggested (2/2 precision 1.0).
        # The higher-coverage qualifying cell wins: 0.45/0.0.
        self.assertEqual(point, (0.45, 0.0))

    def test_evaluate_with_params_is_fixed_point(self) -> None:
        bank = {
            "Ava": [[1.0, 0.0, 0.0]],
            "Bella": [[0.0, 1.0, 0.0]],
        }
        result = evaluate_with_params(
            bank=bank,
            queries=[("Ava", [1.0, 0.0, 0.0]), ("Bella", [0.0, 1.0, 0.0])],
            threshold=0.8,
            margin=0.0,
        )
        self.assertEqual(result["suggestions"], 2)
        self.assertEqual(result["correct"], 2)
        self.assertAlmostEqual(float(result["precision"]), 1.0)
        self.assertAlmostEqual(float(result["coverage"]), 1.0)


class DistributionTests(unittest.TestCase):
    def test_percentiles_are_nearest_rank(self) -> None:
        values = [0.1, 0.2, 0.3, 0.4]
        self.assertAlmostEqual(percentile(values, 0.25), 0.1)
        self.assertAlmostEqual(percentile(values, 0.5), 0.2)
        self.assertAlmostEqual(percentile(values, 0.75), 0.3)

    def test_summary_of_empty_distribution(self) -> None:
        summary = distribution_summary([])
        self.assertEqual(summary["count"], 0)
        self.assertEqual(summary["median"], 0.0)


class ManifestTests(unittest.TestCase):
    def test_loads_and_groups_entries(self) -> None:
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = (
                Path(temporary_directory) / "dataset.json"
            )
            manifest_path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "entries": [
                            {
                                "game": "G",
                                "character": "Zoe",
                                "role": "bank",
                                "path": r"\\nas\G\Zoe.png",
                            },
                            {
                                "game": "G",
                                "character": "Zoe",
                                "role": "query",
                                "path": r"\\nas\G\Zoe2.png",
                            },
                            {
                                "game": "G",
                                "character": "Jill",
                                "role": "bank",
                                "path": r"\\nas\G\Jill.png",
                            },
                        ],
                    }
                ),
                encoding="utf-8",
            )
            entries = load_manifest(manifest_path)
            self.assertEqual(len(entries), 3)
            dataset = manifest_dataset(entries)
            zoe_bank, zoe_query = dataset["G"]["Zoe"]
            self.assertEqual(zoe_bank, [r"\\nas\G\Zoe.png"])
            self.assertEqual(zoe_query, [r"\\nas\G\Zoe2.png"])

    def test_rejects_bank_query_overlap(self) -> None:
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as temporary_directory:
            manifest_path = (
                Path(temporary_directory) / "overlap.json"
            )
            manifest_path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "entries": [
                            {
                                "game": "G",
                                "character": "Zoe",
                                "role": "bank",
                                "path": r"\\nas\G\Zoe.png",
                            },
                            {
                                "game": "G",
                                "character": "Zoe",
                                "role": "query",
                                "path": r"\\nas\G\Zoe.png",
                            },
                        ],
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(ValueError):
                load_manifest(manifest_path)


if __name__ == "__main__":
    unittest.main()
