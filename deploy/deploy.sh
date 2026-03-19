#!/bin/sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"

if [ ! -f "$ENV_FILE" ]; then
  printf "Error: %s not found. Copy .env.example to .env and fill in the values.\n" "$ENV_FILE" >&2
  exit 1
fi

# shellcheck disable=SC1090
. "$ENV_FILE"

# 1. Namespace
kubectl apply -f "$SCRIPT_DIR/namespace.yaml"

# 2. Redis secret (created imperatively to avoid committing secrets)
kubectl create secret generic redis-secret -n lwid \
  --from-literal=password="$REDIS_PASSWORD" \
  --dry-run=client -o yaml | kubectl apply -f -

# 3. Redis deployment + service
kubectl apply -f "$SCRIPT_DIR/redis.yaml"

# 4. Wait for Redis to be ready
printf "Waiting for Redis to be ready...\n"
kubectl rollout status deployment/redis -n lwid --timeout=60s

# 5. JuiceFS secret (created imperatively to avoid committing secrets)
kubectl create secret generic juicefs-secret -n lwid \
  --from-literal=name="$JUICEFS_NAME" \
  --from-literal=metaurl="$JUICEFS_METAURL" \
  --from-literal=storage="$JUICEFS_STORAGE" \
  --from-literal=bucket="$JUICEFS_BUCKET" \
  --from-literal=access-key="$JUICEFS_ACCESS_KEY" \
  --from-literal=secret-key="$JUICEFS_SECRET_KEY" \
  --dry-run=client -o yaml | kubectl apply -f -

# 6. Storage (PV + PVC)
kubectl apply -f "$SCRIPT_DIR/storage.yaml"

# 7. Deployment + Service
kubectl apply -f "$SCRIPT_DIR/deployment.yaml"
kubectl apply -f "$SCRIPT_DIR/service.yaml"

# 8. OVH credentials for cert-manager (in cert-manager namespace)
kubectl create secret generic ovh-credentials-lwid -n cert-manager \
  --from-literal=applicationKey="$OVH_APPLICATION_KEY" \
  --from-literal=applicationSecret="$OVH_APPLICATION_SECRET" \
  --from-literal=consumerKey="$OVH_CONSUMER_KEY" \
  --dry-run=client -o yaml | kubectl apply -f -

# 9. ClusterIssuer + TLS certificate + Ingress
kubectl apply -f "$SCRIPT_DIR/clusterissuer.yaml"
kubectl apply -f "$SCRIPT_DIR/certificate.yaml"
kubectl apply -f "$SCRIPT_DIR/ingress.yaml"

printf "\nDeployed. Check status:\n"
printf "  kubectl get pods -n lwid\n"
printf "  kubectl get certificate -n lwid\n"
printf "  kubectl get ingress -n lwid\n"
