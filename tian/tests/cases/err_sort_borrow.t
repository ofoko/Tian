# &[]T 借用视图不能 sort（规范第 26 节）
f/sz(a:&[]i64):i64{
    sort(a)
    r/0
}
//h=[]i64{3,1}
//z=sz(&h)
` tos(z)
