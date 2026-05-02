KARCH  := aarch64-linux-gnu
CACHE  ?= $(CURDIR)/.cache
SEED   ?= $(HOME)/d/kominka/seed/target/aarch64-unknown-linux-musl/debug/seed

ifneq ($(wildcard $(SEED)),)
# The published tarball has applets as hardlinks; mounting /usr/bin/seed alone
# does not override them. Derive the full applet list from applet_list.rs and
# mount the dev binary at each path. Once the builder image is rebuilt with the
# symlink-based seed package, this reduces to a single -v for /usr/bin/seed.
_SEED_APPLETS := $(shell grep -E 'name: "[^"]+"' $(HOME)/d/kominka/seed/src/applet_list.rs \
    | sed 's/.*name: "\([^"]*\)".*/\1/' | sort -u)
_SEED_MOUNT := $(foreach a,$(_SEED_APPLETS),-v "$(SEED):/usr/bin/$(a):ro")
else
_SEED_MOUNT :=
endif

.PHONY: dev test builder build-all build baseline

builder:
	docker build --build-arg KARCH=$(KARCH) -t kominka:builder .

Makefile: ;

build-all: builder
	scripts/build-all.sh

build: builder
	@: $${PACKAGE:?Usage: make build PACKAGE=<name>}
	ONLY_PKG=$(PACKAGE) scripts/build-all.sh

baseline:
	@latest=$$(ls -t .cache/runs/*.jsonl 2>/dev/null | head -1); \
	if [ -z "$$latest" ]; then echo "No run files in .cache/runs/"; exit 1; fi; \
	cp "$$latest" scripts/build-baseline.jsonl; \
	echo "Baseline set from $$latest"

%: builder
	@mkdir -p $(CACHE)/bin $(CACHE)/src $(CACHE)/sources
	docker run --rm \
		-v "$(CURDIR)/packages:/packages" \
		-v "$(CURDIR)/pm.ysh:/usr/bin/pm:ro" \
		-v "$(CURDIR)/pm.ysh:/packages/pm/pm.ysh:ro" \
		-v "$(CACHE)/bin:/root/.cache/kominka/bin" \
		-v "$(CACHE)/src:/root/.cache/kominka/src" \
		-v "$(CACHE)/sources:/root/.cache/kominka/sources" \
		$(_SEED_MOUNT) \
		-e KOMINKA_REPO='https://kominka.17166969.xyz' \
		-e KOMINKA_PATH=/packages \
		-e KOMINKA_COMPRESS=gz \
		-e KOMINKA_COLOR=0 \
		-e KOMINKA_PROMPT=0 \
		-e KOMINKA_FORCE=1 \
		-e LD_LIBRARY_PATH=/usr/lib \
		-e LOGNAME=root \
		-e HOME=/root \
		kominka:builder /usr/local/bin/ysh /usr/bin/pm b $@

.PHONY: dev test

dev:
	@: $${S3_ENDPOINT:?is not set — source .env first}
	@: $${S3_BUCKET:?is not set — source .env first}
	@: $${S3_ACCESS_KEY_ID:?is not set — source .env first}
	@: $${S3_SECRET_ACCESS_KEY:?is not set — source .env first}
	@: $${DB_PATH:?is not set — source .env first}
	@: $${ALLOWED_USERS:?is not set — source .env first}
	@: $${RP_ID:?is not set — source .env first}
	@: $${RP_ORIGIN:?is not set — source .env first}
	@: $${R2_PUBLIC_URL:?is not set — source .env first}
	cd server && cargo run

test:
	cd server && cargo test

release:
	scripts/build-deb.sh

deploy: release
	scp server/kominka-repo_0.1.0_amd64.deb oracle:
	ssh -T oracle "sh -c 'sudo dpkg -i kominka-repo_0.1.0_amd64.deb && sudo systemctl restart kominka-repo'"

