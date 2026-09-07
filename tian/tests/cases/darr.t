# v4.0 动态数组：字面量/push/索引/len/移动/传参/返回
//a=[]i64{1,2}
push(a, 30)
push(a, 40)
`len(a)
`a[0]
`a[3]
//b=a
push(b, 50)
`len(b)
`b[4]
//i=0
//s=0
w/i<len(b){
    s=s+b[i]
    i=i+1
}
`s
f/total(x:[]i64):i64{
    //i=0
    //t=0
    w/i<len(x){
        t=t+x[i]
        i=i+1
    }
    r/t
}
//c=[]i64{7,8,9}
`total(c)
f/mk():[]i64{
    //r=[]i64{7,8,9}
    r/r
}
//d=mk()
`len(d)
`d[2]
//f1=[]f64{0.5,1.5}
push(f1, 2.5)
`f1[2]+f1[0]
