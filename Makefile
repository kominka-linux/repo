KARCH ?= $(shell uname -m | sed 's/x86_64/x86_64-linux-gnu/;s/aarch64/aarch64-linux-gnu/;s/arm64/aarch64-linux-gnu/')

.PHONY: dev test builder

builder:
	docker build --build-arg KARCH=$(KARCH) -t kominka:builder .

Makefile: ;

%: builder
	docker run --rm \
		-v "$(CURDIR)/packages:/packages:ro" \
		-v "$(CURDIR)/pm.ysh:/usr/bin/pm:ro" \
		-e KOMINKA_REPO='https://kominka.17166969.xyz' \
		-e KOMINKA_PATH=/packages \
		-e KOMINKA_COMPRESS=gz \
		-e KOMINKA_COLOR=0 \
		-e KOMINKA_PROMPT=0 \
		-e KOMINKA_FORCE=1 \
		-e LD_LIBRARY_PATH=/usr/lib \
		-e LOGNAME=root \
		-e HOME=/root \
		kominka:builder /usr/local/bin/ysh -c 'pm b $@'

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

