local result = template.render_string("Hello {{ name }}!", {name = "World"})
assert.eq(result, "Hello World!")
