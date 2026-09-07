use json

//ok="{\"a\":[1,2.5,true,null],\"b\":{\"c\":\"hi\"}}"
validate(&ok)
`"JSON 有效"

//nested="[[[]],{\"x\":-3.5},\"\\\"quoted\\\"\"]"
validate(&nested)
`"嵌套 JSON 有效"
