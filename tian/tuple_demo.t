# v4.5 元组多返回值验收：(count, sum) 一次返回，消除两遍扫描
f/range_stats(n:i64):(i64,i64){
    //count=0
    //sum=0
    //i=1
    w/i<=n{
        count=count+1
        sum=sum+i
        i=i+1
    }
    r/count, sum
}
//c, s = range_stats(100)
`c
`s

# 含 str 元素的元组：所有权按元素接管，块随即释放
f/pair(n:i64):(i64,str){
    r/n, "hello"
}
//num, label = pair(5)
`num
`label

# 元组透传：直接解构调用结果
f/minmax(a:i64, b:i64):(i64,i64){
    r/a, b
}
//lo, hi = minmax(3, 7)
`lo
`hi
