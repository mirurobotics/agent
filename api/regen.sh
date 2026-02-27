#!/bin/bash
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
this_dir=$git_repo_root_dir/api
codegen_dir=$this_dir/codegen

# generate the models
cd "$this_dir"
make gen

# client
codegen_backend_client_models_dir=$codegen_dir/backend-client/src/models
backend_client_models_target_dir=$git_repo_root_dir/libs/openapi-client/src/models

# replace the target model directories with the generated ones
rm -rf "${backend_client_models_target_dir:?}"/*

# copy all the files in the generated models dirs to the target models dirs
if [ ! -d "$backend_client_models_target_dir" ]; then
    mkdir "$backend_client_models_target_dir"
fi
cp -r "$codegen_backend_client_models_dir"/* "$backend_client_models_target_dir"


# server
codegen_agent_server_models_dir=$codegen_dir/agent-server/src/models
agent_server_models_target_dir=$git_repo_root_dir/libs/openapi-server/src/models

# replace the target model directories with the generated ones
rm -rf "${agent_server_models_target_dir:?}"/*

# copy all the files in the generated models dirs to the target models dirs
if [ ! -d "$agent_server_models_target_dir" ]; then
    mkdir "$agent_server_models_target_dir"
fi
cp -r "$codegen_agent_server_models_dir"/* "$agent_server_models_target_dir"
