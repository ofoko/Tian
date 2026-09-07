# struct.t v3.0 结构体验收程序
# 位置式字段 + 命名式字段 + 字段读取 + 字段赋值 + 所有权转移
struct Point{x:i64  y:i64}
struct Person{name:str  age:i64}

# 位置式构造
//p=Point{1,2}
`p.x
`p.y

# 命名式构造
//q=Point{y:10  x:20}
`q.x
`q.y

# 字段赋值
p.x=100
`p.x

# str 字段：所有权移入结构体
//who=Person{name:"天"  age:1}
`who.name
`who.age

# str 字段重新赋值（释放旧值）
who.name="道"
`who.name

# 结构体作为函数参数（所有权移动）
f/getx(p:Point):i64{
    r/p.x
}
`getx(p)

# 结构体作为返回值
f/origin():Point{
    r/Point{0,0}
}
//o=origin()
`o.x
`o.y
