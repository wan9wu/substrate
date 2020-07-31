(module
	(import "env" "gas" (func $gas (param i32)))
	(func (export "deploy"))
	(func (export "call")
        (call $gas (i32.const 42))
    )
)
