# v4.2 []str 动态字符串数组：push/pop/借用视图/copy/移动/传参
//names=[]str{"天", "道"}
push(names, "无穷")
`len(names)
`names[0]
`names[2]
//first=copy(names[0])
`first
//moved=names
push(moved, "终")
`len(moved)
`moved[3]
f/join(a:[]str):str{
    //i=1
    //r=copy(a[0])
    w/i<len(a){
        r=r+"-"+a[i]
        i=i+1
    }
    r/r
}
`join(moved)
