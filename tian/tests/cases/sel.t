//a=5
//b=9
`sel(a<b, a, b)
`sel(a>b, a, b)
`sel(true, 1.5, 2)
//i=0
w/i<3{
    `sel(i==1, 100+i, 0-i)
    i=i+1
}
