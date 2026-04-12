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
