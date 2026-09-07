# sieve.t —— 埃拉托斯特尼素数筛（吃狗粮：[]bool 动态数组）
# 返回 <=n 的素数个数；素数标记保存在 []bool 中（true=质数候选）

f/count_primes(n:i64):i64{
@pre: n>=0
@post: ret>=0
    check(n<100000, "n 过大")
    //s=[]bool{}
    //i=0
    w/i<n{
        push(s, true)
        i=i+1
    }
    //p=2
    w/p*p<n{
        i/s[p]{
            //m=p*p
            w/m<n{
                s[m]=false
                m=m+p
            }
        }
        p=p+1
    }
    //cnt=0
    //j=2
    w/j<n{
        i/s[j]{
            cnt=cnt+1
        }
        j=j+1
    }
    r/cnt
}
