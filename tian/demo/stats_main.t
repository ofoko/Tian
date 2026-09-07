use stats
use json

//doc="{\"name\":\"天语言\",\"scores\":[98,87.5,100],\"meta\":{\"pass\":true,\"tags\":\"v4.4\"}}"
validate(&doc)
`"JSON 有效"
`count_numbers(&doc)
`sum_numbers(&doc)

//bad="{\"x\":1 \"y\":2}"
`count_numbers(&bad)
