# v5.0 generics 快速验证（Result / Option / Map 泛型）
f/div(a:i64, b:i64):Result[i64,str]{
    i/b==0{
        r/Result::Err("除零")
    }
    r/Result::Ok(a/b)
}
f/dbl(o:Option[i64]):i64{
    match o {
        Option::Some(v) => { r/v }
        Option::None => { r/0 }
    }
}
//r1=div(10,2)
match r1 {
    Result::Ok(v) => { ` "Ok="+tos(v) }
    Result::Err(e) => { ` "Err="+e }
}
//r2=div(1,0)
match r2 {
    Result::Ok(v) => { ` "Ok="+tos(v) }
    Result::Err(e) => { ` "Err="+e }
}
//o1:Option[i64]=Option::Some(7)
` "Some->"+tos(dbl(o1))
//o2:Option[i64]=Option::None
` "None->"+tos(dbl(o2))
//m=Map[str,i64]{ "a": 1, "b": 2 }
m["c"]=3
` "map="+tos(m["b"])
` "done"