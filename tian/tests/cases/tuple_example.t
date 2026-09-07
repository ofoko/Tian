# v4.6 收尾 #1：@example 元组结构比较（契约在函数体首部）+ 解构打印，双后端一致
f/range_stats(n:i64):(i64, i64){
@example: range_stats(3) -> (3, 6)
r/ n, n*2
}
f/tp(n:i64):(i64, str){
@example: tp(5) -> (5, "hi")
r/ n, "hi"
}
f/div(a:i64,b:i64):(i64, f64){
@example: div(5, 2) -> (2, 2.5)
r/ a/b, a/2.0
}
//a, b = range_stats(3)
`a
`b
//p, q = tp(2)
`p
`q
//m, k = div(7, 2)
`m
`k
