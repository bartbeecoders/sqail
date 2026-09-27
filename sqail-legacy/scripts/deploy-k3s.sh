#!/bin/bash
set -euo pipefail

#===============================================================================
# SQaiL Portal - K3S Deployment Script (Podman)
#===============================================================================
# Builds and pushes the sqail.io marketing site, then deploys to K3S on VPS.
# Architecture: static Nginx container serving the Vite build
# Namespace: sqail
# Registry:  beecodersregistry.azurecr.io
#
# Usage:
#   ./scripts/deploy-k3s.sh [all|build|push|deploy|status|upload-releases]
#
# Required tools locally:
#   podman, ssh, scp
#===============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

COMMAND="${1:-all}"

REGISTRY="${REGISTRY:-beecodersregistry.azurecr.io}"
NAMESPACE="sqail"
IMAGE="$REGISTRY/sqail-portal"

# VPS target (override via environment variables)
VPS_IP="${VPS_IP:-212.47.77.32}"
VPS_USER="${VPS_USER:-bart}"

VPS_BASE_DIR="${VPS_BASE_DIR:-~/sqail}"
VPS_K8S_DIR="$VPS_BASE_DIR/k8s"

# Version from package.json
APP_VERSION=$(grep -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' sqail.portal/package.json | sed 's/.*"\([^"]*\)"$/\1/')

ssh_vps() {
  local cmd="$1"
  ssh -o StrictHostKeyChecking=accept-new "$VPS_USER@$VPS_IP" "bash -lc $(printf %q "$cmd")"
}

kubectl_vps() {
  local args="$1"
  ssh_vps "if command -v kubectl >/dev/null 2>&1; then kubectl $args; else sudo k3s kubectl $args; fi"
}

check_deps() {
  command -v podman >/dev/null 2>&1 || { echo "podman not found"; exit 1; }
  command -v ssh >/dev/null 2>&1 || { echo "ssh not found"; exit 1; }
  command -v scp >/dev/null 2>&1 || { echo "scp not found"; exit 1; }
}

build_image() {
  echo "Building sqail-portal image v$APP_VERSION..."
  cp "$ROOT_DIR/releases.json" "$ROOT_DIR/sqail.portal/releases.json"
  podman build \
    -t "$IMAGE:latest" \
    -t "$IMAGE:$APP_VERSION" \
    -f "$ROOT_DIR/sqail.portal/Dockerfile" \
    "$ROOT_DIR/sqail.portal"
}

push_image() {
  echo "Logging into registry $REGISTRY..."
  if [[ -n "${REGISTRY_USER:-}" && -n "${REGISTRY_PASSWORD:-}" ]]; then
    podman login -u "$REGISTRY_USER" -p "$REGISTRY_PASSWORD" "$REGISTRY"
  else
    podman login "$REGISTRY"
  fi

  echo "Pushing sqail-portal image..."
  podman push "$IMAGE:latest"
  podman push "$IMAGE:$APP_VERSION"
}

deploy_manifests() {
  echo "Deploying to $VPS_USER@$VPS_IP (namespace: $NAMESPACE)..."

  # Ensure working directory on VPS
  ssh_vps "mkdir -p $VPS_K8S_DIR || (command -v sudo >/dev/null 2>&1 && sudo mkdir -p $VPS_K8S_DIR && sudo chown -R $VPS_USER:$VPS_USER $VPS_BASE_DIR)"

  # Copy manifests
  scp -o StrictHostKeyChecking=accept-new -r "$ROOT_DIR/k8s/portal" "$VPS_USER@$VPS_IP:$VPS_K8S_DIR/"

  # Apply
  kubectl_vps "apply -f $VPS_K8S_DIR/portal/namespace.yaml"

  # Check for image pull secret
  if ! kubectl_vps "-n $NAMESPACE get secret acr-secret >/dev/null 2>&1"; then
    echo "WARNING: Missing secret '$NAMESPACE/acr-secret' (imagePullSecret)."
    echo "         Create it so K3S can pull images from $REGISTRY."
  fi

  kubectl_vps "apply -f $VPS_K8S_DIR/portal/deployment.yaml"

  # Restart to pick up new image
  kubectl_vps "-n $NAMESPACE rollout restart deployment sqail-portal"
  kubectl_vps "-n $NAMESPACE rollout status deployment sqail-portal --timeout=60s"

  echo ""
  echo "Deployed sqail-portal v$APP_VERSION"
  echo "  • NodePort:  http://$VPS_IP:32080"
  echo "  • Ingress:   https://www.sqail.dev"
  echo ""
  echo "NOTE: To serve download binaries, upload them separately:"
  echo "  ./scripts/upload-release.sh"
}

upload_releases() {
  "$ROOT_DIR/scripts/upload-release.sh"
}

status() {
  kubectl_vps "-n $NAMESPACE get pods,svc,ingress"
}

main() {
  check_deps

  case "$COMMAND" in
    all)
      echo "Deploying sqail-portal v$APP_VERSION"
      build_image
      push_image
      deploy_manifests
      status
      ;;
    build)
      build_image
      ;;
    push)
      push_image
      ;;
    deploy)
      deploy_manifests
      ;;
    status)
      status
      ;;
    upload-releases)
      upload_releases
      ;;
    *)
      echo "Unknown command: $COMMAND"
      echo "Usage: $0 [all|build|push|deploy|status|upload-releases]"
      exit 1
      ;;
  esac
}

main
