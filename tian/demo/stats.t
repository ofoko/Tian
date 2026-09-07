# stats.t —— JSON 线性扫描统计器（v4.4 吃狗粮：状态机 + tof + sub）
# 字符类判断：JSON 数字字符 = - + . 0-9
# 注意：不支持科学计数法（e/E 会撞上 true/false 的字母，标量状态机无法区分）
f/is_num_char(c:i64):i64{
    //r=sel(c==45, 1, sel(c==43, 1, sel(c==46, 1, 0)))
    //r2=sel(c>=48, sel(c<=57, 1, 0), 0)
    r/sel(r==1, 1, r2)
}

# 数出 JSON 文本中（字符串外）的数字个数
f/count_numbers(s:&str):i64{
    //i=0
    //n=len(s)
    //in_str=false
    //esc=false
    //cnt=0
    //in_num=false
    w/i<n{
        //c=s[i]
        i/in_str{
            i/esc{
                esc=false
            }e/{
                i/c==92{
                    esc=true
                }e/{
                    i/c==34{
                        in_str=false
                    }
                }
            }
        }e/{
            i/c==34{
                in_str=true
                i/in_num{
                    cnt=cnt+1
                    in_num=false
                }
            }e/{
                i/is_num_char(c)==1{
                    in_num=true
                }e/{
                    i/in_num{
                        cnt=cnt+1
                        in_num=false
                    }
                }
            }
        }
        i=i+1
    }
    i/in_num{
        cnt=cnt+1
    }
    r/cnt
}

# 累加 JSON 文本中（字符串外）的所有数字
f/sum_numbers(s:&str):f64{
    //i=0
    //n=len(s)
    //in_str=false
    //esc=false
    //start=0-1
    //sum=0.0
    w/i<n{
        //c=s[i]
        i/in_str{
            i/esc{
                esc=false
            }e/{
                i/c==92{
                    esc=true
                }e/{
                    i/c==34{
                        in_str=false
                    }
                }
            }
        }e/{
            i/c==34{
                in_str=true
                i/start>=0{
                    sum=sum+tof(sub(s, start, i-start))
                    start=0-1
                }
            }e/{
                i/is_num_char(c)==1{
                    i/start<0{
                        start=i
                    }
                }e/{
                    i/start>=0{
                        sum=sum+tof(sub(s, start, i-start))
                        start=0-1
                    }
                }
            }
        }
        i=i+1
    }
    i/start>=0{
        sum=sum+tof(sub(s, start, n-start))
    }
    r/sum
}
