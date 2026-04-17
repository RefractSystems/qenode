import asyncio

import mcp.types as types
from mcp.server import Server


async def main():
    s = Server("test")

    @s.list_tools()
    async def lt():
        return [types.Tool(name="tool1", description="desc", inputSchema={"type":"object"})]

    handler = s.request_handlers[types.ListToolsRequest]
    req = types.ListToolsRequest(method="tools/list")
    res = await handler(req)
    print("ListTools result:", res)

asyncio.run(main())
