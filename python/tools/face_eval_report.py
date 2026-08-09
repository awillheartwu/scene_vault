"""Builds the SFace vs ArcFace A/B report: a self-contained HTML page plus
an overview PNG for quick comparison."""

from __future__ import annotations

import base64
import io
import json
import statistics

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

import argparse


def load(root: str, name: str) -> dict:
    with open(f"{root}/{name}.json", encoding="utf-8") as handle:
        return json.load(handle)


def overall(report: dict) -> dict:
    return report["overall"]


def pct(value: float) -> str:
    return f"{value:.1%}"


def rank_chart(axes, sface: dict, arcface: dict) -> None:
    labels = ["rank 1", "rank 2", "rank 3", "rank 4+"]
    sf = [overall(sface)["rankDistribution"][key] for key in
          ("rank1", "rank2", "rank3", "rank4Plus")]
    af = [overall(arcface)["rankDistribution"][key] for key in
          ("rank1", "rank2", "rank3", "rank4Plus")]
    width = 0.36
    positions = range(len(labels))
    axes.bar([p - width / 2 for p in positions], sf, width,
             label="SFace", color="#94a3b8")
    axes.bar([p + width / 2 for p in positions], af, width,
             label="ArcFace R50", color="#2563eb")
    for p, value in zip(positions, sf):
        axes.text(p - width / 2, value + 0.4, str(value), ha="center", fontsize=8)
    for p, value in zip(positions, af):
        axes.text(p + width / 2, value + 0.4, str(value), ha="center", fontsize=8)
    axes.set_xticks(list(positions), labels)
    axes.set_ylabel("queries (of 71)")
    axes.set_title("Ground-truth rank distribution\n(higher rank 1 is better)")
    axes.legend(fontsize=8)
    axes.set_ylim(0, 60)


def ecdf(values: list[float]) -> tuple[list[float], list[float]]:
    ordered = sorted(values)
    y = [i / (len(ordered) - 1) for i in range(len(ordered))]
    return ordered, y


def distribution_chart(axes, sface: dict, arcface: dict) -> None:
    for report, label, color, style in (
        (sface, "SFace same", "#16a34a", "-"),
        (sface, "SFace cross", "#dc2626", "--"),
        (arcface, "ArcFace same", "#15803d", "-"),
        (arcface, "ArcFace cross", "#b91c1c", "--"),
    ):
        scores = (
            [s for game in report["games"] for s in game["same_scores"]]
            if label.endswith("same")
            else [s for game in report["games"] for s in game["cross_scores"]]
        )
        x, y = ecdf(scores)
        axes.plot(x, y, label=label, color=color, linestyle=style, linewidth=1.6)
    axes.set_xlabel("cosine similarity")
    axes.set_ylabel("cumulative fraction")
    axes.set_title("Same vs cross score distributions\n(separation = model quality)")
    axes.legend(fontsize=8)


def precision_coverage_chart(axes, sface: dict, arcface: dict) -> None:
    for report, label, color, marker in (
        (sface, "SFace", "#94a3b8", "o"),
        (arcface, "ArcFace R50", "#2563eb", "s"),
    ):
        cells = [
            cell for cell in overall(report)["grid"]
            if cell["topk"] == 1 and cell["suggestions"] >= 3
        ]
        axes.scatter(
            [cell["precision"] for cell in cells],
            [cell["coverage"] for cell in cells],
            s=22,
            color=color,
            marker=marker,
            alpha=0.75,
            label=label,
        )
    axes.axvspan(0.90, 1.0, color="#16a34a", alpha=0.08)
    axes.axvline(0.90, color="#16a34a", linewidth=0.8, linestyle=":")
    axes.axvline(0.95, color="#16a34a", linewidth=0.8, linestyle=":")
    axes.set_xlabel("suggestion precision")
    axes.set_ylabel("coverage (share of queries)")
    axes.set_title(
        "Precision–coverage operating points (topk=1, thr x margin grid)\n"
        "higher curve in the green band = more suggestions at >=90% precision"
    )
    axes.set_xlim(0.3, 1.02)
    axes.legend(fontsize=8)


def per_game_chart(axes, sface: dict, arcface: dict) -> None:
    games_sf = {game["name"]: game for game in sface["games"]}
    games_af = {game["name"]: game for game in arcface["games"]}
    names = sorted(games_sf)
    short = [name.split("_")[0] for name in names]
    sf_top1 = [
        games_sf[name]["top1_hits"] / games_sf[name]["queries"]
        if games_sf[name]["queries"] else 0.0
        for name in names
    ]
    af_top1 = [
        games_af[name]["top1_hits"] / games_af[name]["queries"]
        if games_af[name]["queries"] else 0.0
        for name in names
    ]
    width = 0.36
    positions = range(len(names))
    axes.bar([p - width / 2 for p in positions], sf_top1, width,
             label="SFace", color="#94a3b8")
    axes.bar([p + width / 2 for p in positions], af_top1, width,
             label="ArcFace R50", color="#2563eb")
    axes.set_xticks(list(positions), short, rotation=45, ha="right", fontsize=7)
    axes.set_ylabel("top-1 accuracy")
    axes.set_ylim(0, 1.05)
    axes.set_title("Per-game top-1 accuracy")
    axes.legend(fontsize=8)


def build_charts(sface: dict, arcface: dict) -> bytes:
    figure, axes = plt.subplots(2, 2, figsize=(13.5, 9.5))
    rank_chart(axes[0][0], sface, arcface)
    distribution_chart(axes[0][1], sface, arcface)
    precision_coverage_chart(axes[1][0], sface, arcface)
    per_game_chart(axes[1][1], sface, arcface)
    figure.tight_layout()
    buffer = io.BytesIO()
    figure.savefig(buffer, format="png", dpi=150)
    return buffer.getvalue()


def compare_rows(
    sface: dict, arcface: dict
) -> list[tuple[str, str, str, str, str]]:
    def cells(report: dict) -> tuple[dict, dict]:
        eligible = report.get("overallEligible")
        return overall(report), (eligible or {})

    sf, sf_el = cells(sface)
    af, af_el = cells(arcface)

    def rank_text(report: dict) -> str:
        d = report["rankDistribution"]
        return f"{d['rank1']}/{d['rank2']}/{d['rank3']}/{d['rank4Plus']}"

    rows = [
        ("有效查询（单脸图）", "71 / 56", "71 / 56", "71 / 56", "71 / 56"),
        ("Top-1", pct(sf["top1Accuracy"]), pct(af["top1Accuracy"]),
         pct(sf_el["top1Accuracy"]), pct(af_el["top1Accuracy"])),
        ("Top-3", pct(sf["top3Accuracy"]), pct(af["top3Accuracy"]),
         pct(sf_el["top3Accuracy"]), pct(af_el["top3Accuracy"])),
        (
            "rank 分布 1/2/3/4+",
            rank_text(sf), rank_text(af),
            rank_text(sf_el), rank_text(af_el),
        ),
        ("同角色分数中位", f"{sf['sameScores']['median']:.3f}",
         f"{af['sameScores']['median']:.3f}",
         f"{sf_el['sameScores']['median']:.3f}", f"{af_el['sameScores']['median']:.3f}"),
        ("跨角色分数中位", f"{sf['crossScores']['median']:.3f}",
         f"{af['crossScores']['median']:.3f}",
         f"{sf_el['crossScores']['median']:.3f}", f"{af_el['crossScores']['median']:.3f}"),
        ("最优建议精度（覆盖率）",
         f"{pct(sf['bestSuggestionPrecision'])}（{pct(sf['coverageAtBestPrecision'])}）",
         f"{pct(af['bestSuggestionPrecision'])}（{pct(af['coverageAtBestPrecision'])}）",
         f"{pct(sf_el['bestSuggestionPrecision'])}（{pct(sf_el['coverageAtBestPrecision'])}）",
         f"{pct(af_el['bestSuggestionPrecision'])}（{pct(af_el['coverageAtBestPrecision'])}）"),
        ("精度 ≥90% 时的覆盖率", pct(sf["coverageAt90"]), pct(af["coverageAt90"]),
         pct(sf_el["coverageAt90"]), pct(af_el["coverageAt90"])),
        ("精度 ≥95% 时的覆盖率", pct(sf["coverageAt95"]), pct(af["coverageAt95"]),
         pct(sf_el["coverageAt95"]), pct(af_el["coverageAt95"])),
        ("运行时默认阈值 / margin（provisional）",
         "0.50 / 0.05", "0.50 / 0.10", "0.50 / 0.05", "0.50 / 0.10"),
    ]
    return rows


def build_html(chart_png: bytes, sface: dict, arcface: dict) -> str:
    encoded = base64.b64encode(chart_png).decode("ascii")
    logo = arcface.get("leaveOneGameOut") or {}
    logo_rows = ""
    if logo.get("folds"):
        logo_rows = "".join(
            f"<tr><td>{fold['heldOutGame']}</td>"
            f"<td>{fold['calibratedThreshold']:.2f} / {fold['calibratedMargin']:.2f}</td>"
            f"<td>{pct(fold['eligible']['precision'])}</td>"
            f"<td>{pct(fold['eligible']['coverage'])}</td>"
            f"<td>{pct(fold['eligible']['falseSuggestRate'])}</td>"
            f"<td>{fold['eligible']['suggestions']}/{fold['eligible']['total']}</td></tr>"
            for fold in logo["folds"]
        )
        logo_all = logo.get("overallAll", {})
        logo_el = logo.get("overallEligible", {})
    else:
        logo_all = logo_el = {}
    rows_html = "".join(
        f"<tr><td>{label}</td><td>{sf_all}</td><td class='af'>{af_all}</td>"
        f"<td>{sf_el}</td><td class='af'>{af_el}</td></tr>"
        for label, sf_all, af_all, sf_el, af_el in compare_rows(sface, arcface)
    )
    games_sf = {game["name"]: game for game in sface["games"]}
    games_af = {game["name"]: game for game in arcface["games"]}
    eligible_sf = {game["name"]: game for game in sface["eligibleGames"]}
    eligible_af = {game["name"]: game for game in arcface["eligibleGames"]}
    per_game_html = "".join(
        f"<tr><td>{name}</td><td>{games_sf[name]['queries']}</td>"
        f"<td>{pct(games_sf[name]['top1_hits'] / games_sf[name]['queries']) if games_sf[name]['queries'] else '—'}</td>"
        f"<td class='af'>{pct(games_af[name]['top1_hits'] / games_af[name]['queries']) if games_af[name]['queries'] else '—'}</td>"
        f"<td>{pct(eligible_sf[name]['top1_hits'] / eligible_sf[name]['queries']) if eligible_sf[name]['queries'] else '—'}</td>"
        f"<td class='af'>{pct(eligible_af[name]['top1_hits'] / eligible_af[name]['queries']) if eligible_af[name]['queries'] else '—'}</td></tr>"
        for name in sorted(games_sf)
    )
    return f"""<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8">
<title>Scene Vault · SFace vs ArcFace R50 基准对比</title>
<style>
  body {{ font-family: "Microsoft YaHei", system-ui, sans-serif; margin: 0; background: #f5f6f8; color: #1f2430; }}
  .wrap {{ max-width: 1080px; margin: 0 auto; padding: 28px 20px 60px; }}
  h1 {{ font-size: 22px; margin: 0 0 4px; }}
  .sub {{ color: #6b7280; font-size: 13px; margin-bottom: 22px; }}
  .card {{ background: #fff; border: 1px solid #e5e7eb; border-radius: 12px; padding: 18px 20px; margin-bottom: 18px; box-shadow: 0 1px 3px rgba(0,0,0,.04); }}
  h2 {{ font-size: 15px; margin: 0 0 12px; }}
  table {{ border-collapse: collapse; width: 100%; font-size: 13px; }}
  th, td {{ padding: 8px 10px; border-bottom: 1px solid #eef0f3; text-align: left; }}
  th {{ background: #fafafa; font-weight: 600; }}
  td.af {{ font-weight: 700; color: #1d4ed8; }}
  img {{ max-width: 100%; border-radius: 8px; }}
  .concl li {{ margin: 6px 0; font-size: 13.5px; line-height: 1.6; }}
  .note {{ color: #6b7280; font-size: 12px; margin-top: 8px; }}
</style></head>
<body><div class="wrap">
  <h1>Scene Vault · 人脸识别 A/B：SFace vs ArcFace R50（w600k）</h1>
  <div class="sub">数据集：13 个游戏 / 114 角色 / 283 张图（NAS 清单模式，不复制）· 仅单脸图参与 · 同一 YuNet 检测与同一 evaluator，唯一变量为 recognizer · 2026-08-08</div>

  <div class="card">
    <h2>核心指标对比（全部结果可复现）</h2>
    <table>
      <tr>
        <th>指标</th>
        <th>SFace · 全部 71</th><th class="af">ArcFace · 全部 71</th>
        <th>SFace · 可识别 56</th><th class="af">ArcFace · 可识别 56</th>
      </tr>
      {rows_html}
    </table>
    <div class="note">
      "可识别" = 该角色的 Face Bank 至少 1 个有效样本（回答"模型认不认识已知的人"）；
      "全部" = 所有单脸查询（回答"真实工作流整体能帮我多少"，零样本角色计入沉默/误推荐）。
      头版 Top-1/Top-3/rank 使用产品实际规则（max）；阈值/margin 为网格结果，
      运行时默认值标注 provisional，待 LOOGO 决定。
    </div>
  </div>

  <div class="card">
    <h2>四张关键图</h2>
    <img alt="benchmark charts" src="data:image/png;base64,{encoded}">
  </div>

  <div class="card">
    <h2>逐游戏 Top-1</h2>
    <table>
      <tr>
        <th>游戏</th><th>查询数</th>
        <th>SFace</th><th class="af">ArcFace</th>
        <th>SFace · 可识别</th><th class="af">ArcFace · 可识别</th>
      </tr>
      {per_game_html}
    </table>
    <div class="note">查询数很少的游戏（0004/0008/0089/0091 等 1~2 张）波动大，仅作参考。</div>
  </div>

  <div class="card">
    <h2>Leave-One-Game-Out（ArcFace：每折在其余 12 个游戏上调参，再评第 13 个）</h2>
    <table>
      <tr>
        <th>留出游戏</th><th>校准点 阈值/margin</th>
        <th>精度</th><th>覆盖率</th><th>错误建议率</th><th>建议/查询</th>
      </tr>
      {logo_rows}
    </table>
    <table style="margin-top:12px">
      <tr><th>留出集合（13 折汇总）</th><th>精度</th><th>覆盖率</th><th>错误建议率</th></tr>
      <tr><td>全部查询</td>
        <td>{pct(logo_all.get('precision', 0))}</td>
        <td>{pct(logo_all.get('coverage', 0))}</td>
        <td>{pct(logo_all.get('falseSuggestRate', 0))}</td></tr>
      <tr><td>可识别子集</td>
        <td>{pct(logo_el.get('precision', 0))}</td>
        <td>{pct(logo_el.get('coverage', 0))}</td>
        <td>{pct(logo_el.get('falseSuggestRate', 0))}</td></tr>
    </table>
    <div class="note">
      校准点全部落在 0.45 家族（11 折 margin 0.00、2 折 0.05），说明 ≥95% 精度
      的最优操作点在新游戏上泛化良好（可识别子集 held-out 精度 95.7%、覆盖率
      80.4%）。运行时默认仍保守地保持 0.50/0.10，是否下调 0.45 待更多数据决定。
    </div>
  </div>

  <div class="card">
    <h2>结论</h2>
    <ul class="concl">
      <li><b>模型能力（可识别子集）</b>：SFace Top-1 85.7% → ArcFace <b>91.1%</b>（Top-3 98.2%）；精度 ≥95% 时覆盖率 49.3% → <b>82.1%</b>。</li>
      <li><b>端到端（全部查询）</b>：零样本角色（Face Bank 里还没有这个人）会拉低指标——ArcFace rank4+ 的 16 个错误里 <b>15 个是零样本、只有 1 个有样本</b>；瓶颈是 Face Bank 覆盖而非模型。</li>
      <li><b>LOOGO 泛化</b>：ArcFace 在 12 游戏校准的 ≥95% 操作点，用于第 13 个游戏时仍保持 95.7% 精度、80.4% 覆盖率——说明全局 profile 思路成立。</li>
      <li><b>匹配规则</b>：最优操作点均为 topk=1（max）+ margin，支持"多 prototype 分布"假说。</li>
      <li><b>过程教训</b>：此前 10.4% 的 SFace 结论是 alignCrop 跨进程漂移的假象（已修复并固化为回归测试）；多脸图标签语义问题靠单脸过滤解决。</li>
    </ul>
  </div>
</div></body></html>"""


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Render the SFace vs ArcFace A/B report (HTML + PNG)"
    )
    parser.add_argument(
        "--root",
        default="python/tests/fixtures/face_eval",
        help="directory with sface.json and arcface.json",
    )
    parser.add_argument(
        "--out-dir",
        default=".",
        help="where to write face_eval_report.html and face_eval_overview.png",
    )
    args = parser.parse_args()
    sface = load(args.root, "sface")
    arcface = load(args.root, "arcface")
    chart_png = build_charts(sface, arcface)
    html = build_html(chart_png, sface, arcface)
    with open(f"{args.out_dir}/face_eval_report.html", "w", encoding="utf-8") as handle:
        handle.write(html)
    with open(f"{args.out_dir}/face_eval_overview.png", "wb") as handle:
        handle.write(chart_png)
    print("written:", args.out_dir)


if __name__ == "__main__":
    main()
