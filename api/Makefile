BACKEND_CLIENT_FILE:= specs/backend-client/v01.yaml
BACKEND_CLIENT_CODEGEN_DIR := codegen/backend-client
AGENT_SERVER_FILE := specs/agent-server/v01.yaml
AGENT_SERVER_CODEGEN_DIR := codegen/agent-server
RUST_TEMPLATES_DIR := templates/rust

gen: clean
	make gen-backend-client
	make gen-agent-server

gen-backend-client: clean-backend-client
	openapi-generator-cli generate \
	-i ${BACKEND_CLIENT_FILE} \
	-g rust \
	-t ${RUST_TEMPLATES_DIR} \
	-o ${BACKEND_CLIENT_CODEGEN_DIR} \
	--additional-properties=packageName=openapi-client

gen-agent-server: clean-agent-server
	openapi-generator-cli generate \
	-i ${AGENT_SERVER_FILE} \
	-g rust \
	-t ${RUST_TEMPLATES_DIR} \
	-o ${AGENT_SERVER_CODEGEN_DIR} \
	--additional-properties=packageName=openapi-server

clean: clean-backend-client clean-agent-server 

clean-backend-client:
	rm -rf ${BACKEND_CLIENT_CODEGEN_DIR}

clean-agent-server:
	rm -rf ${AGENT_SERVER_CODEGEN_DIR}
