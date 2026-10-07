# Thin wrapper around the cargo invocations: `make` is not installed everywhere.
.DEFAULT_GOAL := help
.PHONY: help setup test lint fmt fmt-check ci

help: ## show this help
	@grep -E '^[a-z-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m          \033[0m \n", $$1, $$2}'

setup: ## fetch dependencies
	cargo fetch

test: ## full suite, including the HTTP transport
	cargo test --all-targets --all-features

lint: ## clippy, warnings are errors
	cargo clippy --all-targets --all-features -- -D warnings

fmt: ## write formatting
	cargo fmt

fmt-check: ## check formatting without writing
	cargo fmt --check

ci: fmt-check lint test ## everything
