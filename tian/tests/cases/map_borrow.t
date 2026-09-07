# 只读借用 &map[K]V（规范 25.6）
f/total(m:&map[str]i64):i64{
    //s=0
    s=s+m["a"]
    s=s+m["b"]
    s=s+m["a"]
    r/s
}
//m=map[str]i64{}
m["a"]=5
m["b"]=7
` "total="+tos(total(&m))
` "m len="+tos(len(m))
` "has b="+tos(has(m,"b"))