local program_path = assert(arg[1], "expected a generated Lua program path")
local result = dofile(program_path)
if result ~= nil then
  print(result)
end

