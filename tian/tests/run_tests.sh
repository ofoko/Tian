#!/bin/zsh
# 天语言黄金测试运行器
#
# 用法：./tests/run_tests.sh [tc 路径]
#   默认使用 tian/tc/target/release/tc
#
# 规则：
#   - tests/cases/*.t 是黄金测试源码
#   - 同名 .expect 文件是期望的 stdout（不存在则期望运行成功、不比对输出）
#   - 名为 err_*.t 的用例期望"编译失败"（以 --diagnose-json 验证 stage != ok）
#   - 每个正常用例跑双后端（tc run = C 中转；tc build = Cranelift 原生），
#     输出与退出码必须一致——这是双后端语义一致性的护栏
set -u

TC="${1:-$(dirname "$0")/../tc/target/release/tc}"
CASES="$(dirname "$0")/cases"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

pass=0; fail=0
fail_list=()

for src in "$CASES"/*.t(N); do
  name=$(basename "$src" .t)
  expect="$CASES/$name.expect"

  if [[ $name == err_* ]]; then
    # 编译期错误用例：--diagnose-json 必须报错
    if "$TC" "$src" --diagnose-json > "$TMP/diag.json" 2>/dev/null; then
      echo "FAIL $name：期望编译错误，实际编译通过"
      fail=$((fail+1)); fail_list+=("$name"); continue
    fi
    if grep -q '"stage":"ok"' "$TMP/diag.json"; then
      echo "FAIL $name：期望编译错误，诊断输出为 ok"
      fail=$((fail+1)); fail_list+=("$name"); continue
    fi
    pass=$((pass+1)); continue
  fi

  # C 后端（tc run）
  "$TC" run "$src" > "$TMP/c.out" 2> "$TMP/c.err"; cec=$?
  # 原生后端（tc build + 直接运行）
  if ! "$TC" build "$src" > "$TMP/build.log" 2>&1; then
    echo "FAIL $name：tc build 失败"
    cat "$TMP/build.log"
    fail=$((fail+1)); fail_list+=("$name"); continue
  fi
  "$(dirname "$src")/$name" > "$TMP/n.out" 2> "$TMP/n.err"; nec=$?

  if [[ $name == rt_* ]]; then
    # 运行时错误用例：两个后端都必须非零退出，且 stdout/stderr 完全一致（规范第 15 节）
    ok=1
    [[ $cec -ne 0 ]] || { echo "FAIL $name：期望运行时失败，C 后端退出码 0"; ok=0; }
    [[ $nec -ne 0 ]] || { echo "FAIL $name：期望运行时失败，原生后端退出码 0"; ok=0; }
    cmp -s "$TMP/c.out" "$TMP/n.out" || { echo "FAIL $name：双后端 stdout 不一致"; diff "$TMP/c.out" "$TMP/n.out" | head; ok=0; }
    cmp -s "$TMP/c.err" "$TMP/n.err" || { echo "FAIL $name：双后端 stderr 不一致"; diff "$TMP/c.err" "$TMP/n.err" | head; ok=0; }
    if [[ -f "$expect" ]]; then
      cmp -s "$TMP/c.err" "$expect" || { echo "FAIL $name：stderr 与 .expect 不符"; diff "$expect" "$TMP/c.err" | head; ok=0; }
    fi
    [[ $ok -eq 1 ]] && pass=$((pass+1)) || { fail=$((fail+1)); fail_list+=("$name"); }
    continue
  fi

  ok=1
  [[ $cec -eq 0 ]] || { echo "FAIL $name：C 后端退出码 $cec"; cat "$TMP/c.err"; ok=0; }
  [[ $nec -eq 0 ]] || { echo "FAIL $name：原生后端退出码 $nec"; cat "$TMP/n.err"; ok=0; }
  cmp -s "$TMP/c.out" "$TMP/n.out" || { echo "FAIL $name：双后端输出不一致"; diff "$TMP/c.out" "$TMP/n.out" | head; ok=0; }
  if [[ -f "$expect" ]]; then
    cmp -s "$TMP/c.out" "$expect" || { echo "FAIL $name：输出与 .expect 不符"; diff "$expect" "$TMP/c.out" | head; ok=0; }
  fi
  if [[ $ok -eq 1 ]]; then
    # v3.9：fmt 一致性——格式化输出必须仍可运行且结果一致，且 fmt 幂等
    # （fmt 输出放在用例同目录，保证 use 的相对路径解析）
    ftmp="$CASES/__fmt_$name.t"
    "$TC" fmt "$src" > "$ftmp" 2>/dev/null && \
    "$TC" run "$ftmp" > "$TMP/f.out" 2>/dev/null && \
    cmp -s "$TMP/c.out" "$TMP/f.out" && \
    "$TC" fmt "$ftmp" > "$TMP/f2.t" 2>/dev/null && \
    cmp -s "$ftmp" "$TMP/f2.t" || {
      echo "FAIL $name：fmt 输出与原程序行为不一致或 fmt 不幂等"
      ok=0
    }
    rm -f "$ftmp"
  fi
  if [[ $ok -eq 1 ]]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); fail_list+=("$name")
  fi
done

echo
echo "通过 $pass / $((pass+fail))"
if [[ $fail -gt 0 ]]; then
  echo "失败用例：${fail_list[*]}"
  exit 1
fi
