"""
Entry point for the virtmcu MCP server.
"""
import asyncio
import sys
import logging

from tools.mcp_server.server import create_mcp_server

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)


async def main():
    server = create_mcp_server()
    # Run the server using stdio streams
    # We must use stdio as per standard MCP usage for local clients
    
    # Imports inside to avoid loading if not needed
    from mcp.server.stdio import stdio_server

    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream,
            write_stream,
            server.create_initialization_options()
        )

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
    except Exception as e:
        logging.error(f"MCP server crashed: {e}")
        sys.exit(1)
