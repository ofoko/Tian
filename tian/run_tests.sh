#!/usr/bin/env bash
# run_tests.sh —— 天语言一键验收门（docs/ROADMAP.md 第 0 步）
# 每版必跑：单测 → 双后端对拍 → fmt 幂等且可重编译
# 原生构建/格式化均在临时目录进行（不污染仓库中已被跟踪的历史产物）
# 用法: ./run_tests.sh
set -uo pipefail
cd "$(dirname "$0")"
REPO="$PWD"
CARGO="$HOME/.cargo/bin/cargo"
TC="$REPO/tc/target/release/tc"
FAIL=0
FALLBACKS=()

say() { printf '\n== %s ==\n' "$1"; }

# 复制某 .t 及其 use 依赖到临时目录（demo 模块互相依赖，整目录拷贝最稳）
stage() {
    local t="$1" tmpd="$2"
    cp "$REPO/$t" "$tmpd/"
    case "$t" in
        demo/*) cp "$REPO"/demo/*.t "$tmpd/" ;;
    esac
}

# 原生构建 + 运行（临时目录）。程序输出走 stdout；构建日志写 NATIVE_LOG（命令替换是子 shell，变量传不出来）
NATIVE_LOG=$(mktemp)
run_native() {
    local t="$1" tmpd base out blog
    tmpd=$(mktemp -d)
    stage "$t" "$tmpd"
    base=$(basename "${t%.t}")
    blog=$( cd "$tmpd" && "$TC" build "$base.t" 2>&1 ) || { printf '%s' "$blog" > "$NATIVE_LOG"; rm -rf "$tmpd"; return 1; }
    out=$( cd "$tmpd" && "./$base" 2>&1 ) || { printf '%s' "$blog" > "$NATIVE_LOG"; rm -rf "$tmpd"; return 1; }
    printf '%s' "$blog" > "$NATIVE_LOG"
    rm -rf "$tmpd"
    printf '%s' "$out"
}

say "1/4 cargo build --release"
"$CARGO" build --release --manifest-path tc/Cargo.toml --quiet || { echo "构建失败"; exit 1; }
echo "ok"

say "2/4 cargo test"
"$CARGO" test --manifest-path tc/Cargo.toml 2>&1 | grep "test result" || FAIL=1

say "3/4 双后端对拍（C vs Cranelift，输出必须逐字节一致）"
for t in tuple_demo.t main.t own.t borrow.t contract.t struct.t \
         demo/main.t demo/heap.t demo/sieve.t demo/json_main.t demo/stats_main.t; do
    [ -f "$t" ] || { echo "  SKIP（不存在）$t"; continue; }
    if ! out_c=$("$TC" run "$t" 2>&1); then
        echo "  FAIL C 后端: $t"; FAIL=1; continue
    fi
    if ! out_n=$(run_native "$t"); then
        echo "  FAIL 原生后端: $t"; FAIL=1; continue
    fi
    if grep -q "回退" "$NATIVE_LOG"; then
        FALLBACKS+=("$t")
        fb="（原生回退 C，Verifier 待修）"
    else
        fb=""
    fi
    if [ "$out_c" = "$out_n" ]; then
        echo "  PASS $t$fb"
    else
        echo "  FAIL 双后端输出不一致: $t"
        diff <(printf '%s\n' "$out_c") <(printf '%s\n' "$out_n") | head -6
        FAIL=1
    fi
done

say "4/4 tc fmt 幂等 + 格式化后可重编译（独立程序；含 use 的文件在带依赖的临时目录检查）"
tmp1="fmt_1.t"; tmp2="fmt_2.t"
for t in tuple_demo.t main.t own.t demo/main.t; do
    tmpd=$(mktemp -d)
    stage "$t" "$tmpd"
    base=$(basename "$t")
    ( cd "$tmpd" || exit 1
      if ! "$TC" fmt "$base" > "$tmp1" 2>/dev/null; then
          echo "  FAIL fmt 失败: $t"; exit 9
      fi
      "$TC" fmt "$tmp1" > "$tmp2" 2>/dev/null
      if ! diff -q "$tmp1" "$tmp2" >/dev/null; then
          echo "  FAIL fmt 不幂等: $t"; exit 8
      fi
      if ! out_fmt=$("$TC" run "$tmp1" 2>&1); then
          echo "  FAIL fmt 后不可重编译: $t"; exit 7
      fi
      out_raw=$("$TC" run "$base" 2>&1)
      if [ "$out_fmt" != "$out_raw" ]; then
          echo "  FAIL fmt 前后行为不一致: $t"; exit 6
      fi
      echo "  PASS $t"
    ) || FAIL=1
    rm -rf "$tmpd"
done
rm -f "$NATIVE_LOG"

printf '\n'
if [ "${#FALLBACKS[@]}" -gt 0 ]; then
    echo "⚠ 原生后端回退 C（Verifier bug，已登记 v4.6 待修）：${FALLBACKS[*]}"
fi
if [ "$FAIL" -eq 0 ]; then
    echo "全部验收通过 ✔"
else
    echo "存在失败项 ✘"; exit 1
fi
