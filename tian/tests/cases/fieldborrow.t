# v3.1 字段借用：&p.f 只读借用 str 字段，不移动不复制
struct P{name:str  tag:str}
f/shout(s:&str):str{
    r/s+"!"
}
//p=P{name:"天"  tag:"地"}
`shout(&p.name)
`p.name
`shout(&p.tag)
p.name="新"
`shout(&p.name)
