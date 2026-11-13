BACKEND_FILE := specs/backend/v05.yaml
BACKEND_CODEGEN_DIR := codegen/backend
DEVICE_V02_FILE := specs/device/v02.yaml
DEVICE_V02_CODEGEN_DIR := codegen/device-v02
RUST_TEMPLATES_DIR := templates/rust

gen: clean
	make gen-backend
	make gen-device-v02

gen-backend: clean-backend
	npx --yes @openapitools/openapi-generator-cli generate \
	-i ${BACKEND_FILE} \
	-g rust \
	-t ${RUST_TEMPLATES_DIR} \
	-o ${BACKEND_CODEGEN_DIR} \
	--additional-properties=packageName=backend-api

gen-device-v02: clean-device-v02
	npx --yes @openapitools/openapi-generator-cli generate \
	-i ${DEVICE_V02_FILE} \
	-g rust \
	-t ${RUST_TEMPLATES_DIR} \
	-o ${DEVICE_V02_CODEGEN_DIR} \
	--additional-properties=packageName=device-api-v02

clean: clean-backend clean-device-v02

clean-backend:
	rm -rf ${BACKEND_CODEGEN_DIR}

clean-device-v02:
	rm -rf ${DEVICE_V02_CODEGEN_DIR}
