# json.t —— JSON 解析器（v4.9 吃狗粮：enum+match 和类型 → 真 AST）
# 递归下降：将 JSON 文本解析为 Json 枚举 AST（Null/Bool/Num/Str/Arr/Obj）。
# 校验（语法错误即 panic，规范第 21 节）与结构统计共享同一个 Json 内存表示，
# 取代 v4.2 的纯字符串跳转状态机。
#
# 局限：字符串值按原始子串保存，未解码转义（语言不提供 byte→str 原语）。

enum Json{ Null, Bool(bool), Num(f64), Str(str), Arr([]Json), Obj(map[str]Json) }

# ---- 空白：跳过 空格(32)/制表符(9)/换行(10)/回车(13) ----
f/skip_ws(s:&str, i:i64):i64{
    //p=i
    //n=len(s)
    w/p<n{
        //c=s[p]
        i/c==32{
            p=p+1
        }e/{
            i/c==9{
                p=p+1
            }e/{
                i/c==10{
                    p=p+1
                }e/{
                    i/c==13{
                        p=p+1
                    }e/{
                        break
                    }
                }
            }
        }
    }
    r/p
}

# ---- 字符串：s[i]==34（"），返回 (内容, 位置)；转义序列保留在原始子串 ----
f/parse_string(s:&str, i:i64):(str,i64){
@pre: s[i]==34
    //p=i+1
    w/p<len(s){
        i/s[p]==34{
            r/sub(s, i+1, p-i-1), p+1
        }
        i/s[p]==92{
            p=p+2
        }e/{
            p=p+1
        }
    }
    panic("字符串未闭合")
}

# ---- 数字：[-]D+[.D+][e[+-]D+] → (f64, 位置) ----
f/parse_number(s:&str, i:i64):(f64,i64){
    //p=i
    i/s[p]==45{
        p=p+1
    }
    //digits=0
    w/p<len(s){
        //c=s[p]
        //d=sel(c>=48, sel(c<=57, 1, 0), 0)
        i/d==1{
            p=p+1
            digits=1
        }e/{
            break
        }
    }
    i/digits==0{
        panic("数字缺少整数部分")
    }
    # 小数部分
    i/p<len(s){
        i/s[p]==46{
            p=p+1
            w/p<len(s){
                //c=s[p]
                //d=sel(c>=48, sel(c<=57, 1, 0), 0)
                i/d==0{
                    break
                }
                p=p+1
            }
        }
    }
    # 指数部分 e/E [±]D+
    i/p<len(s){
        //ex=sel(s[p]==101, 1, sel(s[p]==69, 1, 0))
        i/ex==1{
            p=p+1
            i/p<len(s){
                i/s[p]==43{
                    p=p+1
                }e/{
                    i/s[p]==45{
                        p=p+1
                    }
                }
            }
            w/p<len(s){
                //c=s[p]
                //d=sel(c>=48, sel(c<=57, 1, 0), 0)
                i/d==0{
                    break
                }
                p=p+1
            }
        }
    }
    r/tof(sub(s, i, p-i)), p
}

# ---- 数组：[] 或 [v, v, ...] → Json::Arr ----
f/parse_array(s:&str, i:i64):(Json,i64){
@pre: s[i]==91
    //p=skip_ws(s, i+1)
    i/s[p]==93{
        //arr=[]Json{}
        r/Json::Arr(arr), p+1
    }
    //arr=[]Json{}
    w/true{
        p=skip_ws(s, p)
        //v, p1 = parse_value(s, p)
        p=p1
        push(arr, v)
        p=skip_ws(s, p)
        i/s[p]==44{
            p=p+1
        }e/{
            i/s[p]==93{
                p=p+1
                //whole=Json::Arr(arr)
                r/whole, p
            }e/{
                panic("数组缺少逗号或右括号")
            }
        }
    }
    r/Json::Null, i
}

# ---- 对象：{} 或 {"k":v, ...} → Json::Obj ----
f/parse_object(s:&str, i:i64):(Json,i64){
@pre: s[i]==123
    //p=skip_ws(s, i+1)
    i/s[p]==125{
        //m=map[str]Json{}
        r/Json::Obj(m), p+1
    }
    //m=map[str]Json{}
    w/true{
        p=skip_ws(s, p)
        i/s[p]!=34{
            panic("对象的键必须是字符串")
        }
        //k, p1 = parse_string(s, p)
        p=p1
        p=skip_ws(s, p)
        i/s[p]!=58{
            panic("对象缺少冒号")
        }
        p=skip_ws(s, p+1)
        //v, p2 = parse_value(s, p)
        p=p2
        m[k]=v
        p=skip_ws(s, p)
        i/s[p]==44{
            p=p+1
        }e/{
            i/s[p]==125{
                p=p+1
                //whole=Json::Obj(m)
                r/whole, p
            }e/{
                panic("对象缺少逗号或右括号")
            }
        }
    }
    r/Json::Null, i
}

# ---- 值分发：按首字节字符识别 ----
f/parse_value(s:&str, i:i64):(Json,i64){
@pre: i<len(s)
    //c=s[i]
    i/c==123{
        //cv, cp = parse_object(s, i)
        r/cv, cp
    }
    i/c==91{
        //av, ap = parse_array(s, i)
        r/av, ap
    }
    i/c==34{
        //v, p = parse_string(s, i)
        r/Json::Str(v), p
    }
    i/c==116{
        check(sub(s, i, 4)=="true", "无效的 true 字面量")
        r/Json::Bool(true), i+4
    }
    i/c==102{
        check(sub(s, i, 5)=="false", "无效的 false 字面量")
        r/Json::Bool(false), i+5
    }
    i/c==110{
        check(sub(s, i, 4)=="null", "无效的 null 字面量")
        r/Json::Null, i+4
    }
    //d=sel(c==45, 1, sel(c>=48, 1, 0))
    i/d==1{
        //nn, pp = parse_number(s, i)
        r/Json::Num(nn), pp
    }
    panic("意外的 JSON 值")
}

# ---- 校验入口：解析整段 JSON，语法错误即 panic，成功返回末尾位置（规范第 21 节） ----
# 校验与结构共享同一 parse_* 递归下降；构建的 Json AST 在本函数返回时确定性深释放。
f/validate(s:&str):i64{
    //v, end = parse_value(s, 0)
    //n=len(s)
    check(end==n, "JSON 结尾有多余字符")
    r/end
}

