# sort(a)：动态数组原地升序排序（规范第 26 节）
# []i64
//a=[]i64{5,1,9,3,7}
sort(a)
` "a len="+tos(len(a))
//i=0
w/i<len(a){
    ` "a["+tos(i)+"]="+tos(a[i])
    i=i+1
}
# []f64
//f=[]f64{2.5,0.5,1.5}
sort(f)
` "f len="+tos(len(f))
//j=0
w/j<len(f){
    ` "f["+tos(j)+"]="+tos(f[j])
    j=j+1
}
# []str
//s=[]str{"pear","apple","banana"}
sort(s)
` "s len="+tos(len(s))
//k=0
w/k<len(s){
    ` "s["+tos(k)+"]="+s[k]
    k=k+1
}
# 整合：sort(keys(m)) → 有序遍历 map（键排序后确定性输出）
//m=map[str]i64{}
m["banana"]=2
m["apple"]=5
m["cherry"]=8
//ks=keys(m)
sort(ks)
` "sorted map keys:"
//p=0
w/p<len(ks){
    ` "  "+ks[p]+" -> "+tos(m[ks[p]])
    p=p+1
}
# 空数组/单元素为幂等空操作
//e=[]i64{}
sort(e)
` "empty len="+tos(len(e))
//one=[]i64{4}
sort(one)
` "one len="+tos(len(one))+", val="+tos(one[0])
