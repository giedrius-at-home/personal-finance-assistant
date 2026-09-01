.PHONY: check format lint security test run compose-config

format:
	uv run ruff format .
	uv run ruff check --fix .

lint:
	uv run ruff format --check .
	uv run ruff check .
	uv run mypy src

security:
	uv run bandit -q -r src
	uv run pip-audit

test:
	uv run pytest --cov=finance_assistant --cov-report=term-missing

check: lint security test

run:
	uv run uvicorn finance_assistant.api:app --reload

compose-config:
	docker compose config --quiet
