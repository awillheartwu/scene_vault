"""Pure scoring helpers for the face-recognition benchmark.

No OpenCV imports here: everything operates on feature vectors, so the
matching math can be unit-tested without the optional vision stack and the
CLI can compute metrics on already-extracted embeddings offline.
"""

from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
import json
from pathlib import Path


def cosine_similarity(
    first: Sequence[float], second: Sequence[float]
) -> float:
    """Cosine between two vectors; 0.0 when lengths differ or a vector is
    zero, mirroring the Rust matching guard."""
    if not first or len(first) != len(second):
        return 0.0
    dot = sum(a * b for a, b in zip(first, second, strict=True))
    first_norm = math.sqrt(sum(a * a for a in first))
    second_norm = math.sqrt(sum(b * b for b in second))
    if first_norm == 0.0 or second_norm == 0.0:
        return 0.0
    return dot / (first_norm * second_norm)


def topk_mean(scores: Sequence[float], k: int) -> float:
    """Mean of the top-k scores (fewer when the list is shorter)."""
    if not scores:
        return 0.0
    if k < 1:
        raise ValueError("k must be at least 1")
    return sum(sorted(scores, reverse=True)[:k]) / min(k, len(scores))


@dataclass(frozen=True, slots=True)
class RankedCharacter:
    character: str
    score: float


def rank_characters(
    query: Sequence[float],
    bank: Mapping[str, Sequence[Sequence[float]]],
    *,
    topk: int = 3,
) -> list[RankedCharacter]:
    """Per-character top-k-mean score over that character's bank samples,
    sorted descending. `topk=1` equals the plain max (most similar
    prototype) rule. Characters without any usable bank sample are excluded:
    they cannot be matched, so ranking them at score 0 would be misleading
    (they can never be suggested)."""
    scored: list[RankedCharacter] = []
    for character, samples in bank.items():
        if not samples:
            continue
        similarities = [
            cosine_similarity(query, sample) for sample in samples
        ]
        scored.append(
            RankedCharacter(
                character=character,
                score=topk_mean(similarities, topk),
            )
        )
    scored.sort(key=lambda entry: entry.score, reverse=True)
    return scored


def suggest(
    query: Sequence[float],
    bank: Mapping[str, Sequence[Sequence[float]]],
    *,
    topk: int = 3,
    threshold: float,
    margin: float,
) -> str | None:
    """Scene Vault's suggestion rule: the top character must clear the score
    threshold AND beat the runner-up by at least `margin`. Preferring no
    suggestion over a confident wrong one.

    The same character must never be compared against its own samples: callers
    with self-match risk must exclude the query's own enrolled sample first.
    """
    ranked = rank_characters(query, bank, topk=topk)
    if not ranked:
        return None
    first = ranked[0]
    if first.score < threshold:
        return None
    second_score = ranked[1].score if len(ranked) > 1 else 0.0
    if first.score - second_score < margin:
        return None
    return first.character


def evaluate_with_params(
    *,
    bank: Mapping[str, Sequence[Sequence[float]]],
    queries: Sequence[tuple[str, Sequence[float]]],
    threshold: float,
    margin: float,
    topk: int = 1,
) -> dict[str, float | int]:
    """Applies a fixed operating point (no tuning) to a query set and returns
    suggestion counts and rates. Used for held-out evaluation in
    leave-one-game-out: the parameters come from the calibration fold and
    are never touched here."""
    suggestions = 0
    correct = 0
    total = len(queries)
    for true_character, query in queries:
        suggested = suggest(
            query, bank, topk=topk, threshold=threshold, margin=margin
        )
        if suggested is None:
            continue
        suggestions += 1
        if suggested == true_character:
            correct += 1
    return {
        "total": total,
        "suggestions": suggestions,
        "correct": correct,
        "precision": correct / suggestions if suggestions else 0.0,
        "coverage": correct / total if total else 0.0,
        "falseSuggestRate": (suggestions - correct) / total if total else 0.0,
    }


def calibrate(
    *,
    bank: Mapping[str, Sequence[Sequence[float]]],
    queries: Sequence[tuple[str, Sequence[float]]],
    topk: int = 1,
    thresholds: Sequence[float] = (0.45, 0.5, 0.55, 0.6, 0.65, 0.7),
    margins: Sequence[float] = (0.0, 0.05, 0.1, 0.15, 0.2),
    min_precision: float = 0.95,
    min_suggestions: int = 3,
) -> tuple[float, float] | None:
    """Grid search over threshold x margin on a CALIBRATION set. Returns the
    operating point with the highest coverage among cells whose precision
    clears `min_precision` (and that produced at least `min_suggestions`
    suggestions), or None when nothing qualifies."""
    best: tuple[float, float, float] | None = None
    for threshold in thresholds:
        for margin in margins:
            result = evaluate_with_params(
                bank=bank,
                queries=queries,
                threshold=threshold,
                margin=margin,
                topk=topk,
            )
            if result["suggestions"] < min_suggestions:
                continue
            if result["precision"] < min_precision:
                continue
            coverage = float(result["coverage"])
            if best is None or coverage > best[0]:
                best = (coverage, threshold, margin)
    if best is None:
        return None
    return best[1], best[2]


@dataclass(frozen=True, slots=True)
class GridEntry:
    topk: int
    threshold: float
    margin: float
    suggestions: int
    correct: int

    @property
    def precision(self) -> float:
        return self.correct / self.suggestions if self.suggestions else 0.0

    @property
    def false_suggest_rate(self) -> float:
        return (self.suggestions - self.correct) / self.total if self.total else 0.0

    @property
    def coverage(self) -> float:
        return self.correct / self.total if self.total else 0.0

    total: int = 0


@dataclass(frozen=True, slots=True)
class GameEvaluation:
    name: str
    characters: int
    bank_samples: int
    queries: int
    skipped_queries: int
    top1_hits: int
    top3_hits: int
    # Counts of the true character's rank (1/2/3/4+), using the same topk=3
    # ranking that produces top1/top3. Bucket 4 also covers queries whose
    # character has no bank samples.
    rank_distribution: tuple[int, int, int, int]
    same_scores: tuple[float, ...]
    cross_scores: tuple[float, ...]
    grid: tuple[GridEntry, ...]


def true_rank(
    query: Sequence[float],
    bank: Mapping[str, Sequence[Sequence[float]]],
    true_character: str,
    *,
    topk: int = 3,
) -> int:
    """1-based rank of `true_character` in the ranked list; 0 when it has no
    bank samples (it cannot be ranked) or does not appear in the ranking."""
    if not bank.get(true_character):
        return 0
    ranked = rank_characters(query, bank, topk=topk)
    return next(
        (
            position
            for position, entry in enumerate(ranked, start=1)
            if entry.character == true_character
        ),
        0,
    )


def evaluate_game(
    *,
    name: str,
    bank: Mapping[str, Sequence[Sequence[float]]],
    queries: Sequence[tuple[str, Sequence[float]]],
    skipped_queries: int = 0,
    topk_values: Sequence[int] = (1, 2, 3),
    thresholds: Sequence[float] = (0.45, 0.5, 0.55, 0.6, 0.65, 0.7),
    margins: Sequence[float] = (0.0, 0.05, 0.1, 0.15, 0.2),
) -> GameEvaluation:
    """Closed-set evaluation of one game: each query is matched against the
    game's bank (train/query disjoint by dataset layout) and scored with the
    max / top2-mean / top3-mean rules across a threshold x margin grid.

    Same/cross score distributions are collected with topk=1 (the most
    similar prototype), which is the distribution the product threshold
    semantics are built on.
    """
    same_scores: list[float] = []
    cross_scores: list[float] = []
    top1_hits = 0
    top3_hits = 0
    rank_counts = [0, 0, 0, 0]
    for true_character, query in queries:
        # Headline metrics use the production matching rule (max =
        # topk_mean(k=1)); the threshold x margin grid still sweeps the
        # aggregation top-k for strategy comparison.
        ranked = rank_characters(query, bank, topk=1)
        if ranked and ranked[0].character == true_character:
            top1_hits += 1
        if any(entry.character == true_character for entry in ranked[:3]):
            top3_hits += 1
        position = true_rank(query, bank, true_character, topk=1)
        if position == 0 or position > 3:
            rank_counts[3] += 1
        else:
            rank_counts[position - 1] += 1
        true_samples = bank.get(true_character, ())
        if true_samples:
            same_scores.append(
                max(
                    cosine_similarity(query, sample)
                    for sample in true_samples
                )
            )
        for character, samples in bank.items():
            if character == true_character or not samples:
                continue
            cross_scores.append(
                max(cosine_similarity(query, sample) for sample in samples)
            )

    grid_entries: list[GridEntry] = []
    for topk in topk_values:
        for threshold in thresholds:
            for margin in margins:
                suggestions = 0
                correct = 0
                for true_character, query in queries:
                    suggested = suggest(
                        query,
                        bank,
                        topk=topk,
                        threshold=threshold,
                        margin=margin,
                    )
                    if suggested is None:
                        continue
                    suggestions += 1
                    if suggested == true_character:
                        correct += 1
                grid_entries.append(
                    GridEntry(
                        topk=topk,
                        threshold=threshold,
                        margin=margin,
                        suggestions=suggestions,
                        correct=correct,
                        total=len(queries),
                    )
                )

    return GameEvaluation(
        name=name,
        characters=len(bank),
        bank_samples=sum(len(samples) for samples in bank.values()),
        queries=len(queries),
        skipped_queries=skipped_queries,
        top1_hits=top1_hits,
        top3_hits=top3_hits,
        rank_distribution=tuple(rank_counts),
        same_scores=tuple(same_scores),
        cross_scores=tuple(cross_scores),
        grid=tuple(grid_entries),
    )


def percentile(values: Sequence[float], quantile: float) -> float:
    """Nearest-rank percentile of a score distribution."""
    if not values:
        return 0.0
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int(math.ceil(quantile * len(ordered))) - 1))
    return ordered[index]


def distribution_summary(
    values: Sequence[float],
) -> dict[str, float]:
    if not values:
        return {
            "count": 0,
            "min": 0.0,
            "p25": 0.0,
            "median": 0.0,
            "p75": 0.0,
            "max": 0.0,
        }
    return {
        "count": len(values),
        "min": min(values),
        "p25": percentile(values, 0.25),
        "median": percentile(values, 0.5),
        "p75": percentile(values, 0.75),
        "max": max(values),
    }


@dataclass(frozen=True, slots=True)
class ManifestEntry:
    game: str
    character: str
    role: str
    path: str


def load_manifest(path: str | Path) -> list[ManifestEntry]:
    """Loads and validates a benchmark manifest (version 1).

    Manifests reference images in place (no copying) for datasets that live
    on a NAS or are too large to duplicate. Validation enforces the same
    train/query separation as the directory layout: one file must never be
    both a bank sample and a query for the same character.
    """
    with open(path, encoding="utf-8") as manifest_file:
        data = json.load(manifest_file)
    if data.get("version") != 1:
        raise ValueError("unsupported manifest version")
    entries: list[ManifestEntry] = []
    for raw in data.get("entries", []):
        entry = ManifestEntry(
            game=str(raw["game"]),
            character=str(raw["character"]),
            role=str(raw["role"]),
            path=str(raw["path"]),
        )
        if entry.role not in ("bank", "query"):
            raise ValueError(f"invalid role {entry.role!r} for {entry.path}")
        if not entry.path:
            raise ValueError("manifest entry has an empty path")
        entries.append(entry)
    by_character: dict[tuple[str, str], dict[str, set[str]]] = {}
    for entry in entries:
        bucket = by_character.setdefault(
            (entry.game, entry.character),
            {"bank": set(), "query": set()},
        )
        bucket[entry.role].add(entry.path)
    overlaps = [
        (character, sorted(bucket["bank"] & bucket["query"]))
        for character, bucket in by_character.items()
        if bucket["bank"] & bucket["query"]
    ]
    if overlaps:
        raise ValueError(
            "the same file must not be both bank and query: "
            + "; ".join(f"{game}/{character}: {paths}" for (game, character), paths in overlaps)
        )
    return entries


def manifest_dataset(
    entries: Sequence[ManifestEntry],
) -> dict[str, dict[str, tuple[list[str], list[str]]]]:
    """Groups manifest entries into game -> character -> (bank paths, query
    paths), ready for feature extraction and evaluation."""
    dataset: dict[str, dict[str, tuple[list[str], list[str]]]] = {}
    for entry in entries:
        bucket = dataset.setdefault(entry.game, {}).setdefault(
            entry.character,
            ([], []),
        )
        if entry.role == "bank":
            bucket[0].append(entry.path)
        else:
            bucket[1].append(entry.path)
    return dataset
