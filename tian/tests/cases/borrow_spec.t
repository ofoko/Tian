# 借用 v2.1 验收程序
//s="天道"
f/greet(x:&str):i64{
    `x
    r/0
}
greet(&s)
`s
# copy(&s) 把借用转为所有权
//c=copy(&s)
`c
