#!/bin/bash
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
this_dir=$git_repo_root_dir/api
codegen_dir=$this_dir/codegen

# generate the models
cd "$this_dir"
make gen

# replace a crate's models with the generated ones
copy_models() {
    local codegen_models_dir=$codegen_dir/$1/src/models
    local target_models_dir=$git_repo_root_dir/libs/$2/src/models

    mkdir -p "$target_models_dir"
    rm -rf "${target_models_dir:?}"/*
    cp -r "$codegen_models_dir"/* "$target_models_dir"
}

copy_models backend backend-api
copy_models device-v02 device-api-v02
copy_models device-v03 device-api-v03
