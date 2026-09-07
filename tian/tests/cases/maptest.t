# 关联数组 map[K]V 全操作黄金测试（规范第 25 节）
# K∈{i64,str}；V∈{i32,i64,f64,bool,str}

//m1=map[str]i64{}
m1["a"]=1
m1["b"]=2
m1["a"]=m1["a"]+10
` "m1 len="+tos(len(m1))
` "m1 a="+tos(m1["a"])
` "m1 b="+tos(m1["b"])
` "has c="+tos(has(m1,"c"))
m1["c"]=3
del(m1,"b")
` "after del b: "+tos(has(m1,"b"))+", len="+tos(len(m1))

# 扩容压力：写入 20 键触发多次 bucket 扩容
//m2=map[str]i64{}
//i=0
w/i<20{
    m2[tos(i)]=i*i
    i=i+1
}
` "m2 len="+tos(len(m2))
` "m2[19]="+tos(m2["19"])

# i64 键 + str 值
//m3=map[i64]str{}
m3[1]="one"
m3[2]="two"
` "m3[2]="+m3[2]
` "m3 has 1="+tos(has(m3,1))

# str 键 + str 值（覆盖更新）
//m4=map[str]str{}
m4["name"]="tian"
m4["name"]="Tian"
` "m4 name="+m4["name"]
` "m4 len="+tos(len(m4))

# f64 / bool 值
//m5=map[str]f64{}
m5["pi"]=3.25
` "m5 pi="+tos(m5["pi"])