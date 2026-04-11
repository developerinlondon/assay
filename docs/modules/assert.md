## assert

Assertion utilities. No `require()` needed. All raise `error()` on failure.

- `assert.eq(a, b, msg?)` — Assert `a == b`
- `assert.ne(a, b, msg?)` — Assert `a ~= b`
- `assert.gt(a, b, msg?)` — Assert `a > b`
- `assert.lt(a, b, msg?)` — Assert `a < b`
- `assert.contains(str, sub, msg?)` — Assert string contains substring
- `assert.not_nil(val, msg?)` — Assert value is not nil
- `assert.matches(str, pattern, msg?)` — Assert string matches regex pattern
