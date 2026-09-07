# join 首个实参非 darr 变量应报错
` cat(join_src(), ",")
f/join_src():[]str{
    r/[]str{"a"}
}
