BACKEND_FILE := specs/backend/v03.yaml
BACKEND_CODEGEN_DIR := codegen/backend
DEVICE_FILE := specs/device/v02.yaml
DEVICE_CODEGEN_DIR := codegen/device
RUST_TEMPLATES_DIR := templates/rust

gen: clean
	make gen-backend
	make gen-device

gen-backend: clean-backend
	openapi-generator-cli generate \
	-i ${BACKEND_FILE} \
	-g rust \
	-t ${RUST_TEMPLATES_DIR} \
	-o ${BACKEND_CODEGEN_DIR} \
	--additional-properties=packageName=backend-api

gen-device: clean-device
	openapi-generator-cli generate \
	-i ${DEVICE_FILE} \
	-g rust \
	-t ${RUST_TEMPLATES_DIR} \
	-o ${DEVICE_CODEGEN_DIR} \
	--additional-properties=packageName=device-api

clean: clean-backend clean-device

clean-backend:
	rm -rf ${BACKEND_CODEGEN_DIR}

clean-device:
	rm -rf ${DEVICE_CODEGEN_DIR}
