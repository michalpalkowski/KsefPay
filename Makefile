.PHONY: help dev deps prod-up prod-down

help:
	@printf "Targets:\n"
	@printf "  make dev      - local dev start (Postgres + Mailpit + server)\n"
	@printf "  make deps     - start local Postgres + Mailpit only\n"
	@printf "  make prod-up  - build and start docker-compose.prod.yml\n"
	@printf "  make prod-down- stop docker-compose.prod.yml\n"

deps:
	@test -f .env || cp .env.example .env
	docker compose up -d postgres mailpit

dev: deps
	cargo run -p ksef-server

prod-up:
	docker compose -f docker-compose.prod.yml up --build

prod-down:
	docker compose -f docker-compose.prod.yml down
