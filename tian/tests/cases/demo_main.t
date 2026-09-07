use demo_heap
use demo_sieve

//h=[]i64{}
h=heap_push(h, 5)
h=heap_push(h, 1)
h=heap_push(h, 9)
h=heap_push(h, 3)
`h[0]
`len(h)
//min=h[0]
h=heap_pop(h)
`min
`h[0]
`len(h)

`count_primes(100)
