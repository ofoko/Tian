# v4.3 &[]T 借用形参：只读视图，不移动所有权，可转借
f/sum(a:&[]i64):i64{
    //i=0
    //s=0
    w/i<len(a){
        s=s+a[i]
        i=i+1
    }
    r/s
}
//h=[]i64{1,2,3}
`sum(&h)
`sum(&h)
`len(h)
`h[0]
f/first(a:&[]str):str{
    r/a[0]
}
//names=[]str{"甲", "乙"}
`first(&names)
`len(names)
f/total2(a:&[]i64, b:&[]i64):i64{
    r/sum(a)+sum(b)
}
//x=[]i64{10,20}
`total2(&h, &x)
f/mutate(a:[]i64):[]i64{
    a[0]=99
    r/a
}
//h2=mutate(h)
`h2[0]
