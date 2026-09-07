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

# keys(m)：键快照 → []K 遍历（规范 25.4）
# m2 键为 str → keys(m2) 为 []str；逐键取回 m2[key] 求和，校验遍历一致性
//kc=keys(m2)
` "m2 keys len="+tos(len(kc))+", match len(m)="+tos(len(kc)==len(m2))
//tot=0
//ki=0
w/ki<len(kc){
    tot=tot+m2[kc[ki]]
    ki=ki+1
}
` "m2 keys sum="+tos(tot)

# m3 键为 i64 → keys(m3) 为 []i64；逐键取回对应值并拼接（键哈希序确定，双后端一致）
//kc3=keys(m3)
` "m3 keys len="+tos(len(kc3))
//kv=""
//k3=0
w/k3<len(kc3){
    kv=kv+m3[kc3[k3]]+" "
    k3=k3+1
}
` "m3 key values: "+kv

# values(m)：值快照 → []V 遍历（规范 25.4）
# m1 值为 i64 → values → []i64；逐元素求和校验一致性
//v1=values(m1)
` "m1 values len="+tos(len(v1))+", match len(m)="+tos(len(v1)==len(m1))
//vt=0
//vi=0
w/vi<len(v1){
    vt=vt+v1[vi]
    vi=vi+1
}
` "m1 values sum="+tos(vt)

# m4 值为 str → values → []str；逐元素拼接校验
//v4=values(m4)
` "m4 values len="+tos(len(v4))
//vv=""
//vi2=0
w/vi2<len(v4){
    vv=vv+v4[vi2]+" "
    vi2=vi2+1
}
` "m4 values: "+vv

# m5 值为 f64 → values → []f64
//v5=values(m5)
` "m5 values len="+tos(len(v5))+", pi="+tos(v5[0])

# bool 值 → values → []bool
//m6=map[str]bool{}
m6["raining"]=true
m6["sunny"]=false
//vb=values(m6)
` "m6 values len="+tos(len(vb))
//bi=0
w/bi<len(vb){
    ` "m6v["+tos(bi)+"]="+tos(vb[bi])
    bi=bi+1
}
