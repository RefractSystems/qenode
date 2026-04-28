import pytest_asyncio

from tools.testing.virtmcu_test_suite.conftest_core import *  # noqa: F403  # noqa: F403
from tools.testing.virtmcu_test_suite.transport import UnixTransportImpl, ZenohTransportImpl


@pytest_asyncio.fixture
async def _sim_transport_zenoh(zenoh_router, zenoh_session):
    transport = ZenohTransportImpl(zenoh_router, zenoh_session)
    await transport.start()
    yield transport
    await transport.stop()

@pytest_asyncio.fixture
async def _sim_transport_unix():
    transport = UnixTransportImpl()
    await transport.start()
    yield transport
    await transport.stop()

@pytest_asyncio.fixture(params=["zenoh", "unix"])
async def sim_transport(request, _sim_transport_zenoh, _sim_transport_unix):
    if request.param == "zenoh":
        yield _sim_transport_zenoh
    else:
        yield _sim_transport_unix
