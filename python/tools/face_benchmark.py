#!/usr/bin/env python3
"""Closed-set face-recognition benchmark for Scene Vault game screenshots.

Dataset layout (train/query disjoint, enforced by the directory structure):

    <root>/<game>/<character>/bank/*.png     -- Face Bank samples
    <root>/<game>/<character>/query/*.png    -- images to recognize

Every query is matched only against its own game's bank (a project is a
game), with the max / top2-mean / top3-mean rules across a threshold x margin
grid. The report contains top-1 / top-3, suggestion precision / coverage /
false-suggest rate per grid cell, and the same/cross score distributions
used to pick the product thresholds. Requires the optional vision stack
(opencv-contrib-python-headless <5, numpy, pillow).

Two dataset modes:

* --data-root: directory layout (train/query separated by folders);
* --manifest: JSON manifest referencing existing images in place, built by
  scripts/build_face_eval_manifest.ps1 from character-named screenshot
  folders (no copying, so NAS-sized datasets work).

Example:

    python tools/face_benchmark.py \
        --data-root D:\\face_eval \
        --yunet-model D:\\models\\face_detection_yunet_2023mar.onnx \
        --sface-model D:\\models\\face_recognition_sface_2021dec.onnx \
        --out face_eval_report.json

    python tools/face_benchmark.py \
        --manifest python\\tests\\fixtures\\face_eval\\dataset.json \
        --yunet-model D:\\models\\face_detection_yunet_2023mar.onnx \
        --sface-model D:\\models\\face_recognition_sface_2021dec.onnx \
        --out face_eval_report.json
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import asdict
from pathlib import Path
from typing import Any

# Limit BLAS/OpenMP threading before any heavy import; the default keeps the
# machine responsive (a benchmark run must not freeze the desktop).
_THREAD_LIMIT = int(os.environ.get("SCENE_VAULT_BENCH_THREADS", "8"))
if _THREAD_LIMIT > 0:
    os.environ["OMP_NUM_THREADS"] = str(_THREAD_LIMIT)
    os.environ["OPENBLAS_NUM_THREADS"] = str(_THREAD_LIMIT)
    os.environ["MKL_NUM_THREADS"] = str(_THREAD_LIMIT)

from scene_vault_ai.eval import (
    GameEvaluation,
    calibrate,
    distribution_summary,
    evaluate_with_params,
    evaluate_game,
    load_manifest,
    manifest_dataset,
    rank_characters,
)
from scene_vault_ai.vision.config import YuNetConfig
from scene_vault_ai.vision.detector import YuNetFaceDetector
from scene_vault_ai.vision.recognizer import (
    ArcfaceConfig,
    ArcfaceFeatureExtractor,
    SfaceConfig,
    SfaceFeatureExtractor,
)

SUPPORTED_EXTENSIONS = {".png", ".jpg", ".jpeg", ".webp", ".bmp"}


def limit_cpu(threads: int) -> None:
    """Binds the process to `threads` logical CPUs (the lowest ids, which are
    the P-cores on hybrid Intel parts) and caps OpenCV's internal thread
    pool. A no-op on failure; `threads <= 0` leaves the machine untouched."""
    if threads <= 0:
        return
    try:
        import cv2

        cv2.setNumThreads(threads)
    except Exception:
        pass
    if sys.platform == "win32":
        try:
            import ctypes

            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            current = ctypes.c_size_t()
            system = ctypes.c_size_t()
            kernel32.GetProcessAffinityMask(
                kernel32.GetCurrentProcess(),
                ctypes.byref(current),
                ctypes.byref(system),
            )
            available = int(system.value)
            mask = 0
            bit = 1
            picked = 0
            while picked < threads and bit <= available:
                if available & bit:
                    mask |= bit
                    picked += 1
                bit <<= 1
            kernel32.SetProcessAffinityMask(
                kernel32.GetCurrentProcess(),
                ctypes.c_size_t(mask),
            )
        except Exception:
            pass


def _images(directory: Path) -> list[Path]:
    if not directory.is_dir():
        return []
    return sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in SUPPORTED_EXTENSIONS
    )


def discover_dataset(
    root: Path,
) -> dict[str, dict[str, tuple[list[Path], list[Path]]]]:
    """game -> character -> (bank images, query images)."""
    dataset: dict[str, dict[str, tuple[list[Path], list[Path]]]] = {}
    for game in sorted(path for path in root.iterdir() if path.is_dir()):
        characters: dict[str, tuple[list[Path], list[Path]]] = {}
        for character in sorted(path for path in game.iterdir() if path.is_dir()):
            bank = _images(character / "bank")
            query = _images(character / "query")
            if bank or query:
                characters[character.name] = (bank, query)
        if characters:
            dataset[game.name] = characters
    return dataset


def extract_features(
    images: list[Path],
    detector: YuNetFaceDetector,
    extractor: object,
) -> tuple[list[Path], list[list[float]], int]:
    """Returns (paths, feature vectors, skipped images). The path list stays
    aligned with the feature list so query details can reference the exact
    source image. Skipping is per image: an unreadable file, a missing face
    or an extraction error never aborts the benchmark."""
    import cv2
    import numpy as np

    kept_paths: list[Path] = []
    features: list[list[float]] = []
    skipped = 0
    for image_path in images:
        # np.fromfile + imdecode instead of cv2.imread: the screenshots live
        # under non-ASCII paths (Chinese game/character names) and imread
        # cannot open those on Windows.
        image_bytes = np.fromfile(image_path, dtype=np.uint8)
        if image_bytes.size == 0:
            print(f"[skip] cannot read {image_path}", file=sys.stderr)
            skipped += 1
            continue
        image_bgr = cv2.imdecode(image_bytes, cv2.IMREAD_COLOR)
        if image_bgr is None:
            print(f"[skip] cannot decode {image_path}", file=sys.stderr)
            skipped += 1
            continue
        faces = detector.detect_all_faces(image_bgr)
        if not faces:
            print(f"[skip] no face in {image_path}", file=sys.stderr)
            skipped += 1
            continue
        if len(faces) > 1:
            # The filename labels the scene's character; with several faces
            # the primary face may be someone else, so the label is
            # ambiguous. Scene Vault's product semantic is one primary
            # character per screenshot, so the benchmark only evaluates
            # single-face images.
            print(
                f"[skip] multi-face ({len(faces)}) in {image_path}",
                file=sys.stderr,
            )
            skipped += 1
            continue
        try:
            features.append(extractor.extract(image_bgr, faces[0]))
            kept_paths.append(image_path)
        except Exception as error:
            print(f"[skip] extraction failed for {image_path}: {error}", file=sys.stderr)
            skipped += 1
    return kept_paths, features, skipped


def merge_games(games: list[GameEvaluation]) -> dict[str, Any]:
    """Overall numbers across games: hits summed, distributions and the
    threshold x margin grid aggregated."""
    total = sum(game.queries for game in games)
    top1 = sum(game.top1_hits for game in games)
    top3 = sum(game.top3_hits for game in games)
    same = [score for game in games for score in game.same_scores]
    cross = [score for game in games for score in game.cross_scores]
    grid: dict[tuple[int, float, float], dict[str, int]] = {}
    for game in games:
        for entry in game.grid:
            key = (entry.topk, entry.threshold, entry.margin)
            slot = grid.setdefault(
                key,
                {"suggestions": 0, "correct": 0},
            )
            slot["suggestions"] += entry.suggestions
            slot["correct"] += entry.correct
    grid_rows = []
    for (topk, threshold, margin), slot in sorted(grid.items()):
        suggestions = slot["suggestions"]
        correct = slot["correct"]
        grid_rows.append(
            {
                "topk": topk,
                "threshold": threshold,
                "margin": margin,
                "suggestions": suggestions,
                "correct": correct,
                "precision": correct / suggestions if suggestions else 0.0,
                "coverage": correct / total if total else 0.0,
                "falseSuggestRate": (suggestions - correct) / total if total else 0.0,
            }
        )
    rank_distribution = [
        sum(game.rank_distribution[index] for game in games)
        for index in range(4)
    ]

    def precision_metrics(
        min_precision: float,
    ) -> dict[str, float]:
        candidates = [
            row
            for row in grid_rows
            if row["precision"] >= min_precision
        ]
        if not candidates:
            return {"precision": 0.0, "coverage": 0.0}
        best = max(candidates, key=lambda row: row["coverage"])
        return {"precision": best["precision"], "coverage": best["coverage"]}

    meaningful = [row for row in grid_rows if row["suggestions"] >= 3]
    best_cell = max(meaningful, key=lambda row: row["precision"]) if meaningful else None
    return {
        "totalQueries": total,
        "top1Accuracy": top1 / total if total else 0.0,
        "top3Accuracy": top3 / total if total else 0.0,
        "rankDistribution": {
            "rank1": rank_distribution[0],
            "rank2": rank_distribution[1],
            "rank3": rank_distribution[2],
            "rank4Plus": rank_distribution[3],
        },
        "sameScores": distribution_summary(same),
        "crossScores": distribution_summary(cross),
        "grid": grid_rows,
        "bestSuggestionPrecision": (
            best_cell["precision"] if best_cell else 0.0
        ),
        "coverageAtBestPrecision": (
            best_cell["coverage"] if best_cell else 0.0
        ),
        "precisionAt90": precision_metrics(0.90)["precision"],
        "coverageAt90": precision_metrics(0.90)["coverage"],
        "precisionAt95": precision_metrics(0.95)["precision"],
        "coverageAt95": precision_metrics(0.95)["coverage"],
    }


def print_game_summary(game: GameEvaluation) -> None:
    total = game.queries or 1
    print(
        f"[{game.name}] characters={game.characters} "
        f"bankSamples={game.bank_samples} queries={game.queries} "
        f"skipped={game.skipped_queries} "
        f"top1={game.top1_hits / total:.1%} "
        f"top3={game.top3_hits / total:.1%}"
    )


def print_best_configs(overall: dict[str, Any], limit: int = 6) -> None:
    candidates = [
        row
        for row in overall["grid"]
        if row["falseSuggestRate"] <= 0.02 and row["suggestions"] > 0
    ]
    if not candidates:
        print("No grid cell reaches a false-suggest rate <= 2%.")
        return
    print("\nBest configs (false-suggest <= 2%, by coverage):")
    for row in sorted(
        candidates, key=lambda entry: entry["coverage"], reverse=True
    )[:limit]:
        print(
            f"  topk={row['topk']} threshold={row['threshold']:.2f} "
            f"margin={row['margin']:.2f} -> "
            f"precision={row['precision']:.1%} "
            f"coverage={row['coverage']:.1%} "
            f"falseSuggest={row['falseSuggestRate']:.1%}"
        )


def check_content_leak(
    dataset: dict[str, dict[str, tuple[list[Path], list[Path]]]],
) -> list[str]:
    """SHA-256 based bank/query leak guard: a file copied and renamed must
    still be caught. Returns human-readable errors, one per duplicate pair."""
    import hashlib

    errors: list[str] = []
    for game, characters in dataset.items():
        bank_hashes: dict[str, str] = {}
        query_hashes: dict[str, str] = {}
        for character, (bank_paths, query_paths) in characters.items():
            for path in bank_paths:
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                bank_hashes.setdefault(digest, str(path))
            for path in query_paths:
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                query_hashes.setdefault(digest, str(path))
        for digest, bank_path in bank_hashes.items():
            if digest in query_hashes:
                errors.append(
                    f"{game}: bank {bank_path} has identical content to "
                    f"query {query_hashes[digest]}"
                )
    return errors


def run_leave_one_game_out(
    game_data: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    """Leave-one-game-out calibration/validation: for each held-out game,
    calibrate threshold/margin on the other 12 games' eligible queries
    (max coverage at >=95% precision, topk=1), then evaluate the held-out
    game with that fixed point — both on all of its queries and on the
    eligible subset."""
    names = sorted(game_data)
    folds: list[dict[str, Any]] = []
    for held_out in names:
        calibration_bank: dict[str, list[list[float]]] = {}
        calibration_queries: list[tuple[str, list[float]]] = []
        for other in names:
            if other == held_out:
                continue
            for character, samples in game_data[other]["bank"].items():
                calibration_bank.setdefault(character, []).extend(samples)
            calibration_queries.extend(game_data[other]["eligible_queries"])
        point = calibrate(
            bank=calibration_bank,
            queries=calibration_queries,
            min_precision=0.95,
        )
        held = game_data[held_out]
        fold: dict[str, Any] = {
            "heldOutGame": held_out,
            "calibratedThreshold": None,
            "calibratedMargin": None,
        }
        for key, queries in (
            (
                "all",
                [
                    (character, feature)
                    for character, feature, _ in held["all_queries"]
                ],
            ),
            ("eligible", held["eligible_queries"]),
        ):
            if point is None:
                fold[key] = {
                    "total": len(queries),
                    "suggestions": 0,
                    "correct": 0,
                    "precision": 0.0,
                    "coverage": 0.0,
                    "falseSuggestRate": 0.0,
                }
            else:
                fold[key] = evaluate_with_params(
                    bank=held["bank"],
                    queries=queries,
                    threshold=point[0],
                    margin=point[1],
                )
        if point is not None:
            fold["calibratedThreshold"] = point[0]
            fold["calibratedMargin"] = point[1]
        folds.append(fold)

    def aggregate(key: str) -> dict[str, float | int]:
        total = sum(int(fold[key]["total"]) for fold in folds)
        suggestions = sum(int(fold[key]["suggestions"]) for fold in folds)
        correct = sum(int(fold[key]["correct"]) for fold in folds)
        return {
            "total": total,
            "suggestions": suggestions,
            "correct": correct,
            "precision": correct / suggestions if suggestions else 0.0,
            "coverage": correct / total if total else 0.0,
            "falseSuggestRate": (
                (suggestions - correct) / total if total else 0.0
            ),
        }

    return {
        "folds": folds,
        "overallAll": aggregate("all"),
        "overallEligible": aggregate("eligible"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Closed-set face-recognition benchmark on Scene Vault game screenshots"
    )
    parser.add_argument(
        "--data-root",
        help="dataset root with <game>/<character>/bank+query folders",
    )
    parser.add_argument(
        "--manifest",
        help="dataset manifest JSON referencing images in place (see "
        "scripts/build_face_eval_manifest.ps1); mutually exclusive with "
        "--data-root",
    )
    parser.add_argument(
        "--threads",
        type=int,
        default=_THREAD_LIMIT,
        help="cap CPU usage to this many logical CPUs (0 = unlimited); "
        "defaults to the SCENE_VAULT_BENCH_THREADS env var or 8",
    )
    parser.add_argument("--yunet-model", required=True, help="YuNet ONNX path")
    parser.add_argument("--sface-model", required=True, help="SFace ONNX path")
    parser.add_argument(
        "--recognizer",
        choices=("sface", "arcface"),
        default="sface",
        help="recognition embedding to evaluate (default: sface)",
    )
    parser.add_argument(
        "--arcface-model",
        help="ArcFace ONNX path (required with --recognizer arcface)",
    )
    parser.add_argument(
        "--leave-one-game-out",
        action="store_true",
        help="run leave-one-game-out calibration/validation after the main "
        "evaluation",
    )
    parser.add_argument(
        "--skip-leak-check",
        action="store_true",
        help="skip the SHA-256 bank/query content leak guard",
    )
    parser.add_argument("--out", help="write the JSON report to this path")
    args = parser.parse_args()
    limit_cpu(args.threads)

    if bool(args.data_root) == bool(args.manifest):
        parser.error("exactly one of --data-root or --manifest is required")
    dataset: dict[str, dict[str, tuple[list[Path], list[Path]]]]
    dataset_root: str | None
    if args.manifest:
        entries = load_manifest(args.manifest)
        dataset_root = None
        raw_dataset = manifest_dataset(entries)
        dataset = {
            game: {
                character: (
                    [Path(path) for path in bank],
                    [Path(path) for path in query],
                )
                for character, (bank, query) in characters.items()
            }
            for game, characters in raw_dataset.items()
        }
        print(
            f"Manifest: games={len(dataset)} "
            f"characters={sum(len(c) for c in dataset.values())} "
            f"entries={len(entries)}"
        )
    else:
        root = Path(args.data_root)
        if not root.is_dir():
            parser.error(f"data root is not a directory: {root}")
        dataset_root = str(root)
        dataset = discover_dataset(root)
        if not dataset:
            parser.error(
                "no game/character/bank+query directories found under the data root"
            )
        for game, characters in dataset.items():
            for character, (bank, query) in characters.items():
                bank_names = {path.name for path in bank}
                overlap = bank_names & {path.name for path in query}
                if overlap:
                    print(
                        f"[warn] {game}/{character}: {len(overlap)} file(s) appear in "
                        "both bank/ and query/; identical files leak the label into "
                        "the bank and inflate accuracy.",
                        file=sys.stderr,
                    )

    if not args.skip_leak_check:
        leak_errors = check_content_leak(dataset)
        if leak_errors:
            for error in leak_errors[:10]:
                print(f"[leak] {error}", file=sys.stderr)
            parser.error(
                f"{len(leak_errors)} bank/query content duplicate(s) found; "
                "fix the dataset before evaluating"
            )

    yunet_model = Path(args.yunet_model)
    sface_model = Path(args.sface_model)
    if not yunet_model.is_file():
        parser.error(f"YuNet model not found: {yunet_model}")
    if not sface_model.is_file():
        parser.error(f"SFace model not found: {sface_model}")
    detector = YuNetFaceDetector(
        YuNetConfig(model_path=yunet_model)
    )
    if args.recognizer == "arcface":
        if not args.arcface_model:
            parser.error("--arcface-model is required with --recognizer arcface")
        arcface_model = Path(args.arcface_model)
        if not arcface_model.is_file():
            parser.error(f"ArcFace model not found: {arcface_model}")
        extractor = ArcfaceFeatureExtractor(ArcfaceConfig(arcface_model))
    else:
        extractor = SfaceFeatureExtractor(SfaceConfig(sface_model))

    game_data: dict[str, dict[str, Any]] = {}
    feature_dim: int | None = None
    for game_name, characters in dataset.items():
        bank: dict[str, list[list[float]]] = {}
        queries: list[tuple[str, list[float], Path]] = []
        skipped = 0
        for character_name, (bank_images, query_images) in characters.items():
            _, bank_features, skipped_bank = extract_features(
                bank_images, detector, extractor
            )
            bank[character_name] = bank_features
            skipped += skipped_bank
            query_paths, query_features, skipped_query = extract_features(
                query_images, detector, extractor
            )
            skipped += skipped_query
            queries.extend(
                (character_name, feature, path)
                for feature, path in zip(
                    query_features, query_paths, strict=True
                )
            )
        if feature_dim is None:
            first_feature = next(
                (
                    feature
                    for features in bank.values()
                    for feature in features
                ),
                None,
            )
            if first_feature is None and queries:
                first_feature = queries[0][1]
            if first_feature is not None:
                feature_dim = len(first_feature)
        game_data[game_name] = {
            "bank": bank,
            "all_queries": queries,
            "eligible_queries": [
                (character, feature)
                for character, feature, _ in queries
                if bank.get(character)
            ],
            "skipped": skipped,
        }

    # Standard evaluation, reported twice: all single-face queries (product
    # semantics, including characters with no usable bank samples) and the
    # eligible subset (GT bank >= 1 sample, answering "does the model
    # recognize people the Face Bank actually knows").
    games_all: list[GameEvaluation] = []
    games_eligible: list[GameEvaluation] = []
    query_details: list[dict[str, Any]] = []
    for game_name, data in game_data.items():
        game_all = evaluate_game(
            name=game_name,
            bank=data["bank"],
            queries=[
                (character, feature)
                for character, feature, _ in data["all_queries"]
            ],
            skipped_queries=data["skipped"],
        )
        games_all.append(game_all)
        games_eligible.append(
            evaluate_game(
                name=game_name,
                bank=data["bank"],
                queries=data["eligible_queries"],
                skipped_queries=data["skipped"],
            )
        )
        print_game_summary(game_all)
        for true_character, feature, query_path in data["all_queries"]:
            ranked = rank_characters(feature, data["bank"], topk=1)
            position = next(
                (
                    index
                    for index, entry in enumerate(ranked, start=1)
                    if entry.character == true_character
                ),
                0,
            )
            query_details.append(
                {
                    "game": game_name,
                    "character": true_character,
                    "queryPath": str(query_path),
                    "eligible": bool(data["bank"].get(true_character)),
                    "trueRank": position,
                    "top5": [
                        {
                            "character": entry.character,
                            "score": round(entry.score, 4),
                        }
                        for entry in ranked[:5]
                    ],
                }
            )

    overall = merge_games(games_all)
    overall_eligible = merge_games(games_eligible)
    for label, report in (
        ("ALL queries", overall),
        ("ELIGIBLE (GT bank >= 1)", overall_eligible),
    ):
        ranks = [
            detail["trueRank"]
            for detail in query_details
            if detail["trueRank"] > 0
            and (label.startswith("ALL") or detail["eligible"])
        ]
        median_rank = sorted(ranks)[len(ranks) // 2] if ranks else 0
        report["medianTrueRank"] = median_rank
        print(
            f"\nOVERVIEW [{label}] queries={report['totalQueries']} "
            f"top1={report['top1Accuracy']:.1%} "
            f"top3={report['top3Accuracy']:.1%}"
        )
        distribution = report["rankDistribution"]
        print(
            f"  true-rank 1/2/3/4+: "
            f"{distribution['rank1']}/{distribution['rank2']}/"
            f"{distribution['rank3']}/{distribution['rank4Plus']} "
            f"(median {median_rank})"
        )
        print(
            "  same median "
            f"{report['sameScores']['median']:.3f} | cross median "
            f"{report['crossScores']['median']:.3f}"
        )
        print(
            "  suggestion quality: best precision "
            f"{report['bestSuggestionPrecision']:.1%} "
            f"(coverage {report['coverageAtBestPrecision']:.1%}); "
            f"precision>=90% -> coverage {report['coverageAt90']:.1%}; "
            f"precision>=95% -> coverage {report['coverageAt95']:.1%}"
        )
    print_best_configs(overall)

    logo: dict[str, Any] | None = None
    if args.leave_one_game_out:
        logo = run_leave_one_game_out(game_data)
        print("\nLEAVE-ONE-GAME-OUT (calibrate on 12, evaluate on 1)")
        for fold in logo["folds"]:
            held = fold["eligible"]
            point = (
                f"{fold['calibratedThreshold']:.2f}/{fold['calibratedMargin']:.2f}"
                if fold["calibratedThreshold"] is not None
                else "none"
            )
            print(
                f"  {fold['heldOutGame']}: point={point} -> "
                f"precision={held['precision']:.1%} "
                f"coverage={held['coverage']:.1%} "
                f"false={held['falseSuggestRate']:.1%} "
                f"(n={held['suggestions']}/{held['total']})"
            )
        for key, label in (
            ("overallAll", "ALL"),
            ("overallEligible", "ELIGIBLE"),
        ):
            held = logo[key]
            print(
                f"  HELD-OUT [{label}]: precision={held['precision']:.1%} "
                f"coverage={held['coverage']:.1%} "
                f"false={held['falseSuggestRate']:.1%} "
                f"(n={held['suggestions']}/{held['total']})"
            )

    if args.out:
        report = {
            "model": {
                "id": extractor.MODEL_ID,
                "version": extractor.MODEL_VERSION,
                "dim": feature_dim,
            },
            "datasetRoot": dataset_root,
            "games": [asdict(game) for game in games_all],
            "eligibleGames": [asdict(game) for game in games_eligible],
            "queryDetails": query_details,
            "overall": overall,
            "overallEligible": overall_eligible,
            "leaveOneGameOut": logo,
        }
        with open(args.out, "w", encoding="utf-8") as output:
            json.dump(report, output, ensure_ascii=False, indent=2)
        print(f"\nReport written to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
