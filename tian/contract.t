# 语义锚点 v2.2 验收程序
f/add(x:i64,y:i64):i64{
    @pre: x>0
    @post: ret>x
    @example: add(1,2) -> 3
    r/x+y
}
f/make(s:str):str{
    @pre: s!=""
    @post: ret=="天道"
    @example: make("天") -> "天道"
    r/s+"道"
}
`add(1,2)
`make("天")
