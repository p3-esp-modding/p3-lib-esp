#!/usr/bin/env bash
# Sube main a la rama remota pre-release (dispara el workflow de pre-release).
# No hay que cambiar de rama ni hacer merges: empuja main tal cual.
set -e
cd "$(dirname "$0")"

if [ -n "$(git status --porcelain)" ]; then
    echo "ERROR: hay cambios sin commitear; haz commit primero."
    exit 1
fi

echo "== Subiendo main =="
git push origin main

echo "== Subiendo main -> pre-release =="
git push origin main:pre-release

echo "OK: workflow de pre-release disparado (ver Actions en GitHub)."
