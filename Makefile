.PHONY: backup bootstrap check codex-tools compose-config demo doctor down format \
	install-macos-app lint logs pull purge-research-cache restore run security setup \
	status stop test test-dashboard upstream-contracts up versions

setup:
	uv sync --all-extras --dev
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

test-dashboard:
	node --test tests/javascript/dashboard.test.cjs

check: lint security test test-dashboard

.PHONY: test-bank-importer
test-bank-importer:
	uv run python scripts/check_bank_worker.py

run:
	uv run uvicorn --app-dir src finance_assistant.api:app --reload

demo:
	PFA_DEMO_MODE=true uv run uvicorn --app-dir src finance_assistant.api:app \
		--host 127.0.0.1 --port 8002

install-macos-app:
	test -f .env
	uv run python scripts/macos_app.py

codex-tools:
	test -f .env
	@docker compose --env-file .env exec -T \
		-e PFA_CODEX_TOOLS_ENABLED=true gateway \
		python -m finance_assistant.mcp_server

purge-research-cache:
	test -f .env
	docker compose --env-file .env exec -T gateway \
		python -m finance_assistant.research_cache_cli --confirm-purge

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
