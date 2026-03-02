#!/bin/bash
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
this_dir=$git_repo_root_dir/api
codegen_dir=$this_dir/codegen

# generate the models
cd "$this_dir"
make gen

# backend
codegen_backend_models_dir=$codegen_dir/backend/src/models
backend_models_target_dir=$git_repo_root_dir/libs/backend-api/src/models

# replace the target model directories with the generated ones
rm -rf "${backend_models_target_dir:?}"/*

# copy all the files in the generated models dirs to the target models dirs
if [ ! -d "$backend_models_target_dir" ]; then
    mkdir "$backend_models_target_dir"
fi
cp -r "$codegen_backend_models_dir"/* "$backend_models_target_dir"


# device
codegen_device_models_dir=$codegen_dir/device/src/models
device_models_target_dir=$git_repo_root_dir/libs/device-api/src/models

# replace the target model directories with the generated ones
rm -rf "${device_models_target_dir:?}"/*

# copy all the files in the generated models dirs to the target models dirs
if [ ! -d "$device_models_target_dir" ]; then
    mkdir "$device_models_target_dir"
fi
cp -r "$codegen_device_models_dir"/* "$device_models_target_dir"
