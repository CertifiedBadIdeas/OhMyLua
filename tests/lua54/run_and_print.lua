local program_path = assert(arg[1], "expected a generated Lua program path")
local chunk = assert(loadfile(program_path))
local result = chunk()
io.write(tostring(result), "\n")
