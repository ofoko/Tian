# catcount.t —— 类目频次聚合（吃狗粮：关联数组 map[K]V）
# 统计一批商品类目出现次数，演示插入/更新/查询/删除/长度
# 惯例：改型函数接管 map 所有权，修改后 return 归还（移动语义）

f/record(m:map[str]i64, cat:str):map[str]i64{
    i/has(m, cat){
        m[cat]=m[cat]+1
    }e/{
        m[cat]=1
    }
    r/m
}

//counts=map[str]i64{}
counts=record(counts, "electronics")
counts=record(counts, "clothing")
counts=record(counts, "electronics")
counts=record(counts, "food")
` "electronics -> "+tos(counts["electronics"])
` "clothing -> "+tos(counts["clothing"])
` "food -> "+tos(counts["food"])
` "total categories: "+tos(len(counts))
` "has books: "+tos(has(counts, "books"))
del(counts, "clothing")
` "after del clothing: "+tos(has(counts, "clothing"))+", total "+tos(len(counts))

# keys(counts) → []str 键快照，遍历打印全部类目及频次
//cats=keys(counts)
` "--- all categories (keys traversal) ---"
//ci=0
w/ci<len(cats){
    ` cats[ci]+" -> "+tos(counts[cats[ci]])
    ci=ci+1
}
` "categories via keys(): "+tos(len(cats))
