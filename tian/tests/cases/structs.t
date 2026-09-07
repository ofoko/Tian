# 结构体：声明/构造/字段读写/所有权/函数
struct P{x:i64  y:i64}
struct N{name:str  v:i64}
//p=P{1,2}
`p.x+p.y
//q=P{y:10  x:20}
`q.x
p.x=100
`p.x
f/gety(p:P):i64{
    r/p.y
}
`gety(p)
f/mk():P{
    r/P{x:7  y:8}
}
//m=mk()
`m.x
`m.y
//n=N{name:"天"  v:1}
n.v=n.v+41
`n.v
`n.name
n.name="道"
`n.name
