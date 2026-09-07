# cat(a, sep)：[]str 以 sep 连接成新 str（规范第 26 节）
# 基础连接
//names=[]str{"甲","乙","丙"}
` "joined: "+cat(names, ",")
` "with-dash: "+cat(names, " - ")
# 空分隔符
` "nospace: "+cat(names, "")
# key 单个元素/空数组
//one=[]str{"only"}
` "one: "+cat(one, ",")
//e=[]str{}
` "empty: ["+cat(e, ",")+"]"
# 整合：cat(keys(m), sep) 需先快照到变量（join 实参须 darr 变量）
# 用 map 键排序 + join 打印逗号间隔类目
//m=map[str]i64{}
m["b"]=2
m["a"]=5
m["c"]=8
//ks=keys(m)
sort(ks)
` "cats: "+cat(ks, ", ")
# 与 tos 组合：数值数组经值快照 join（[]str 值）
//m2=map[str]str{}
m2["x"]="quick"
m2["y"]="brown"
m2["z"]="fox"
//vs=values(m2)
sort(vs)
` "phrase: "+cat(vs, " ")
# 分隔符为新构造的 str（sub 结果）
//sep2=sub(", ", 0, 1)
` "shortsep: "+cat(names, sep2)
