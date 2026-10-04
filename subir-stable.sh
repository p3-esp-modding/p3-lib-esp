#!/usr/bin/env bash
# Sube main a la rama remota stable (dispara el workflow de stable).
# No hay que cambiar de rama ni hacer merges: empuja main tal cual.
set -e
cd "$(dirname "$0")"

if [ -n "$(git status --porcelain)" ]; then
    echo "ERROR: hay cambios sin commitear; haz commit primero."
    exit 1
fi

echo "== Subiendo main =="
git push origin main

echo "== Subiendo main -> stable =="
git push origin main:stable

echo "OK: workflow de stable disparado (crea/actualiza el release estable)."
