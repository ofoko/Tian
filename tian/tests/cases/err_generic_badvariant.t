# 期望编译错误：泛型构造变体与期望实例不匹配（Result 无 Foo 变体）
f/nope():Result[i64,str]{
    r/Result::Foo(1)
}