#!/usr/bin/env bash
set -euxo pipefail

echo "==> Bringing up pendulum docker-compose"
docker compose -f docker/docker-compose.yml -f worlds/pendulum.yml up -d

echo "==> Waiting for coordinator to process quanta..."
sleep 10

# We expect to see 'Quantum 10 complete' at minimum within 10 seconds.
if docker compose -f docker/docker-compose.yml -f worlds/pendulum.yml logs deterministic-coordinator | grep "Quantum 10 complete"; then
    echo "==> SUCCESS: Deterministic coordinator is advancing quanta correctly."
else
    echo "==> ERROR: Did not see quantum advancement!"
    docker compose -f docker/docker-compose.yml -f worlds/pendulum.yml logs
    docker compose -f docker/docker-compose.yml -f worlds/pendulum.yml down -v
    exit 1
fi

echo "==> Tearing down pendulum docker-compose"
docker compose -f docker/docker-compose.yml -f worlds/pendulum.yml down -v

echo "==> Test passed!"
