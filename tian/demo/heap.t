# heap.t —— 二叉最小堆（吃狗粮：动态数组 + 移动语义 + 模块）
# 惯例：修改型函数接管数组所有权，修改后原样交还（返回即归还）

f/sift_up(h:[]i64, i:i64):[]i64{
    //c=i
    w/c>0{
        //p=(c-1)/2
        i/h[p]>h[c]{
            //t=h[p]
            h[p]=h[c]
            h[c]=t
            c=p
        }e/{
            break
        }
    }
    r/h
}

f/heap_push(h:[]i64, v:i64):[]i64{
    push(h, v)
    //n=len(h)
    r/sift_up(h, n-1)
}

f/sift_down(h:[]i64, i:i64, n:i64):[]i64{
    //c=i
    w/true{
        //l=2*c+1
        //r=2*c+2
        //sm=c
        i/l<n{
            i/h[l]<h[sm]{
                sm=l
            }
        }
        i/r<n{
            i/h[r]<h[sm]{
                sm=r
            }
        }
        i/sm==c{
            break
        }
        //t=h[c]
        h[c]=h[sm]
        h[sm]=t
        c=sm
    }
    r/h
}

# 弹出最小值：调用前先读 h[0]，弹出后数组不含它
f/heap_pop(h:[]i64):[]i64{
@pre: len(h)>0
    //last=len(h)-1
    h[0]=h[last]
    pop(h)
    //n=len(h)
    r/sift_down(h, 0, n)
}
