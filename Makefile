IMAGE    := ghcr.io/marlinski/lwid-server
GIT_HASH := $(shell git rev-parse --short HEAD)

.PHONY: build push deploy all

## Build Docker image
build:
	docker build --build-arg GIT_HASH=$(GIT_HASH) -t $(IMAGE):latest -t $(IMAGE):$(GIT_HASH) .

## Push image to GHCR
push:
	docker push $(IMAGE):latest
	docker push $(IMAGE):$(GIT_HASH)

## Build and push
all: build push
