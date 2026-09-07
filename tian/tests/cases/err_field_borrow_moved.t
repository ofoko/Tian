struct P{name:str}
f/f1(s:&str):i64{ r/1 }
//p=P{name:"x"}
//q=p
f1(&p.name)
