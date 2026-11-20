SHELL := /bin/bash

PROJECT_NAME := $(shell sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | head -1 | tr -d '[:space:]')
PROJECT_VERSION := $(shell sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | sed -n '2p' | tr -d '[:space:]')
ifeq ($(PROJECT_NAME),)
    $(error Error: PROJECT file not found or invalid)
endif

TOP_DIR := $(CURDIR)
CARGO := cargo
DISPLAY ?= :1
RUN_WITH ?= nixVulkan
EXAMPLE ?= gizmo

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info ------------------------------------------)

.PHONY: build b compile c run r test t check fmt bench clean help h

build:
	@$(CARGO) build --example $(EXAMPLE)

b: build

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run:
	@DISPLAY=$(DISPLAY) $(RUN_WITH) $(CARGO) run --release --example $(EXAMPLE)

r: run

test:
	@$(CARGO) test

t: test

check:
	@$(CARGO) check --all-targets

fmt:
	@$(CARGO) fmt --all

bench:
	@$(CARGO) bench

clean:
	@$(CARGO) clean

help:
	@echo
	@echo "Usage: make [target]"
	@echo
	@echo "Targets: build / compile / run / test / check / fmt / bench / clean"
	@echo "Examples:"
	@echo "  make run"
	@echo "  make run EXAMPLE=other"
	@echo "  make run DISPLAY=:0"
	@echo "  make run RUN_WITH="
	@echo
h: help
