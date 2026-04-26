.PHONY: all build-frontend clean

all: build-frontend

build-frontend:
	npm install
	npx esbuild static/js/auth.ts static/js/settings.ts --bundle --outdir=static/js --platform=browser --target=es2020

clean:
	rm -rf node_modules
	rm -f static/js/auth.js static/js/settings.js
