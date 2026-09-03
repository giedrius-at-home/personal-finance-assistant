.PHONY: backup bootstrap check compose-config demo doctor down format lint logs \
	pull restore run security setup status stop test upstream-contracts up versions

setup:
	uv sync
	@if [ "$$(uname -s)" = "Darwin" ]; then chflags -R nohidden .venv; fi
	uv run python -c "import finance_assistant"

format:
	uv run ruff format .
	uv run ruff check --fix .

lint:
	uv run ruff format --check .
	uv run ruff check .
	uv run mypy src scripts

security:
	uv run bandit -q -r src scripts
	uv run pip-audit

test:
	uv run pytest --cov=finance_assistant --cov-report=term-missing

upstream-contracts:
	PFA_REQUIRE_UPSTREAM_CONTRACTS=1 uv run pytest -m upstream_contract \
		tests/integration/test_upstream_contracts.py

check: lint security test

run:
	uv run uvicorn --app-dir src finance_assistant.api:app --reload

demo:
	PFA_DEMO_MODE=true uv run uvicorn --app-dir src finance_assistant.api:app \
		--host 127.0.0.1 --port 8002

compose-config:
	docker compose --env-file .env.example config --quiet

bootstrap:
	uv run python scripts/manage.py bootstrap

pull:
	test -f .env
	docker compose --env-file .env pull --ignore-buildable

up:
	test -f .env
	docker compose --env-file .env up --build --detach

status:
	test -f .env
	docker compose --env-file .env ps

doctor:
	uv run python scripts/manage.py doctor

logs:
	test -f .env
	docker compose --env-file .env logs --tail=200

stop:
	test -f .env
	docker compose --env-file .env stop

down:
	test -f .env
	docker compose --env-file .env down

versions:
	test -f .env
	docker compose --env-file .env images

backup:
	uv run python scripts/manage.py backup

restore:
	test -n "$(BACKUP)"
	uv run python scripts/manage.py restore "$(BACKUP)" --confirm-restore
