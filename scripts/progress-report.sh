#!/usr/bin/env bash
# Progress stats: per-day entry/category/commit table + category totals.
# Usage: bash scripts/progress-report.sh [--days N]   (default 14)
# Data sources: docs/progress/YYYY-MM-DD.md entry headings + git rev-list.
set -euo pipefail
cd "$(dirname "$0")/.."

days=14
if [[ "${1:-}" == "--days" && -n "${2:-}" && "$2" =~ ^[0-9]+$ ]]; then
  days="$2"
fi

echo "# Progress 报告 · 近 ${days} 日"
echo
echo "| 日期 | 条目 | 分类分布 | 提交 |"
echo "|---|---|---|---|"

# Newest first: i=0 is today.
for i in $(seq 0 $((days - 1))); do
  d=$(date -d "-${i} days" +%Y-%m-%d)
  f="docs/progress/${d}.md"
  [[ -f "$f" ]] || continue
  entries=$(grep -cE '^## P[0-9]+ · ' "$f" || true)
  cats=$(grep -E '^## P[0-9]+ · [0-9]{2}:[0-9]{2} · [a-z]+ ·' "$f" \
    | awk -F' · ' '{print $3}' | sort | uniq -c \
    | awk '{printf "%s×%s ", $2, $1}')
  commits=$(git rev-list --count HEAD --since "${d} 00:00:00" --until "${d} 23:59:59")
  echo "| ${d} | ${entries:-0} | ${cats:--} | ${commits} |"
done

echo
echo "## 分类总计（全部日志）"
echo
echo "| 分类 | 条目 |"
echo "|---|---|"
cat docs/progress/[0-9]*.md 2>/dev/null \
  | grep -E '^## P[0-9]+ · [0-9]{2}:[0-9]{2} · [a-z]+ ·' \
  | awk -F' · ' '{c[$3]++} END {for (k in c) printf "%s %s\n", k, c[k]}' \
  | sort | awk '{printf "| `%s` | %s |\n", $1, $2}'
