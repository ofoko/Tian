# json.t —— JSON 校验器（v4.2 吃狗粮：str 字节读 / sub / 借用形参 / check / sel / 递归）
# 失败策略：任何语法错误直接 panic（规范第 21 节哲学）

f/skip_ws(s:&str, i:i64):i64{
    //p=i
    //n=len(s)
    w/p<n{
        //c=s[p]
        //ws=sel(c==32, 1, sel(c==9, 1, sel(c==10, 1, sel(c==13, 1, 0))))
        i/ws==0{
            break
        }
        p=p+1
    }
    r/p
}

f/skip_string(s:&str, i:i64):i64{
@pre: s[i]==34
    //p=i+1
    w/p<len(s){
        i/s[p]==34{
            r/p+1
        }
        i/s[p]==92{
            p=p+2
        }e/{
            p=p+1
        }
    }
    panic("字符串未闭合")
}

f/skip_digits(s:&str, i:i64):i64{
    //p=i
    //n=len(s)
    w/p<n{
        //c=s[p]
        //d=sel(c>=48, sel(c<=57, 1, 0), 0)
        i/d==0{
            break
        }
        p=p+1
    }
    r/p
}

f/skip_number(s:&str, i:i64):i64{
    //p=skip_digits(s, i+1)
    i/p<len(s){
        i/s[p]==46{
            r/skip_digits(s, p+1)
        }
    }
    r/p
}

f/skip_array(s:&str, i:i64):i64{
@pre: s[i]==91
    //p=skip_ws(s, i+1)
    i/s[p]==93{
        r/p+1
    }
    w/true{
        p=skip_ws(s, p)
        p=skip_value(s, p)
        p=skip_ws(s, p)
        i/s[p]==44{
            p=p+1
        }e/{
            i/s[p]==93{
                r/p+1
            }e/{
                panic("数组缺少逗号或右括号")
            }
        }
    }
    r/p
}

f/skip_object(s:&str, i:i64):i64{
@pre: s[i]==123
    //p=skip_ws(s, i+1)
    i/s[p]==125{
        r/p+1
    }
    w/true{
        p=skip_ws(s, p)
        i/s[p]!=34{
            panic("对象的键必须是字符串")
        }
        p=skip_string(s, p)
        p=skip_ws(s, p)
        i/s[p]!=58{
            panic("对象缺少冒号")
        }
        p=skip_ws(s, p+1)
        p=skip_value(s, p)
        p=skip_ws(s, p)
        i/s[p]==44{
            p=p+1
        }e/{
            i/s[p]==125{
                r/p+1
            }e/{
                panic("对象缺少逗号或右括号")
            }
        }
    }
    r/p
}

f/skip_value(s:&str, i:i64):i64{
    //c=s[i]
    i/c==34{
        r/skip_string(s, i)
    }
    i/c==123{
        r/skip_object(s, i)
    }
    i/c==91{
        r/skip_array(s, i)
    }
    i/c==116{
        check(sub(s, i, 4)=="true", "无效的 true 字面量")
        r/i+4
    }
    i/c==102{
        check(sub(s, i, 5)=="false", "无效的 false 字面量")
        r/i+5
    }
    i/c==110{
        check(sub(s, i, 4)=="null", "无效的 null 字面量")
        r/i+4
    }
    i/c==45{
        r/skip_number(s, i)
    }
    i/c>=48{
        i/c<=57{
            r/skip_number(s, i)
        }
    }
    panic("意外的字符")
}

f/validate(s:&str):i64{
    //end=skip_value(s, 0)
    //n=len(s)
    check(end==n, "JSON 结尾有多余字符")
    r/end
}
