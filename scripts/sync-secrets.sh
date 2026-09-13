#!/usr/bin/env bash
# 将 config/.env.<profile> 中的变量同步到 GitHub Actions 环境 secret。
#
# 用法：
#   ./scripts/sync-secrets.sh <profile>
#
# 示例：
#   ./scripts/sync-secrets.sh production
#   ./scripts/sync-secrets.sh test

set -euo pipefail

PROFILE="${1:-}"
if [[ -z "$PROFILE" ]]; then
  echo "用法：./scripts/sync-secrets.sh <production|test>" >&2
  exit 1
fi

REPO="axxxx202401/broadcast"
ENV_FILE="$(cd "$(dirname "$0")/.." && pwd)/config/.env.${PROFILE}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "找不到环境文件：$ENV_FILE" >&2
  exit 1
fi

echo "📂 $ENV_FILE"

count=0
while IFS= read -r line || [[ -n "$line" ]]; do
  # 跳过空行和注释
  line="${line%%#*}"          # 去掉行内注释
  line="${line#"${line%%[![:space:]]*}"}"  # 去左空白
  [[ -z "$line" ]] && continue
  [[ "$line" == "export "* ]] && line="${line#export }"

  # 解析 KEY=VALUE
  key="${line%%=*}"
  value="${line#*=}"
  key="${key#"${key%%[![:space:]]*}"}"
  key="${key%"${key##*[![:space:]]}"}"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"

  # 去掉首尾引号
  if [[ ${#value} -ge 2 ]]; then
    first="${value:0:1}"
    last="${value: -1}"
    if [[ "$first" == '"' && "$last" == '"' ]] || [[ "$first" == "'" && "$last" == "'" ]]; then
      value="${value:1:${#value}-2}"
    fi
  fi

  # 只处理合法的环境变量名
  if [[ ! "$key" =~ ^[A-Z_][A-Z0-9_]*$ ]]; then
    continue
  fi

  gh secret set "$key" --repo "$REPO" --env "$PROFILE" --body "$value"
  echo "  ✅ $key"
  ((count++)) || true
done < "$ENV_FILE"

echo "✅ 共同步 $count 个变量到 environment \"$PROFILE\""
