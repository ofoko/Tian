# 只读借用：&str 形参，不移动所有权
f/show(s:&str):str{
    r/s+"!"
}
//s="天"
`show(&s)
`s
`show("字面量")
