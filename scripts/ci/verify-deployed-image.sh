#!/usr/bin/env bash
# Assert that a live workload runs the image the pipeline meant to deploy.
#
#   verify-deployed-image.sh <namespace> <kind/name> <image> [pull-policy]
#
# Example:
#   verify-deployed-image.sh willywompas deploy/willywompas-writer \
#     ghcr.io/e-lemongrab/willywompas-writer:abc123
#
# `helm --wait` already tells you the rollout became ready. What it cannot tell
# you is whether what became ready is what you asked for: a values file that
# pins its own image.tag, a --set that silently lost to a values entry, or a
# chart default, all produce a green deploy of the wrong thing. This closes that
# gap, and is deliberately separate from the deploy itself so it can be called
# after one helm release or five, or after a plain kubectl apply.
#
# pull-policy defaults to Always. Pass "" to skip that check.
set -euo pipefail

if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
  echo "usage: $0 <namespace> <kind/name> <image> [pull-policy]" >&2
  exit 64
fi

NAMESPACE="$1"
WORKLOAD="$2"
EXPECTED_IMAGE="$3"
EXPECTED_POLICY="${4-Always}"

if ! kubectl get "$WORKLOAD" -n "$NAMESPACE" >/dev/null 2>&1; then
  echo "FAIL: $WORKLOAD not found in namespace $NAMESPACE" >&2
  exit 1
fi

# Same pod-template path for Deployment, StatefulSet and Job.
actual_image="$(kubectl get "$WORKLOAD" -n "$NAMESPACE" \
  -o jsonpath='{.spec.template.spec.containers[*].image}')"
actual_policy="$(kubectl get "$WORKLOAD" -n "$NAMESPACE" \
  -o jsonpath='{.spec.template.spec.containers[*].imagePullPolicy}')"

echo "== Deployed image check =="
echo "workload=$NAMESPACE/$WORKLOAD"
echo "expected=$EXPECTED_IMAGE"
echo "actual  =$actual_image"

failed=0

# Multi-container workloads report every image space-separated; the expected one
# has to be among them rather than equal to the whole string.
found=0
for image in $actual_image; do
  [ "$image" = "$EXPECTED_IMAGE" ] && found=1
done
if [ "$found" -ne 1 ]; then
  echo "FAIL: $WORKLOAD does not run $EXPECTED_IMAGE" >&2
  failed=1
fi

if [ -n "$EXPECTED_POLICY" ]; then
  echo "policy  =$actual_policy (expected $EXPECTED_POLICY)"
  case " $actual_policy " in
    *" $EXPECTED_POLICY "*) ;;
    *)
      echo "FAIL: imagePullPolicy is not $EXPECTED_POLICY" >&2
      failed=1
      ;;
  esac
fi

# A workload can carry the right image and still never start it. Surface the
# pull failures explicitly, because they are invisible to `helm --wait` once the
# release has already been marked deployed, and to a plain `kubectl apply`
# always.
pods="$(kubectl get pods -n "$NAMESPACE" \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.status.containerStatuses[*].state.waiting.reason}{"\n"}{end}' \
  2>/dev/null | grep -E 'ImagePullBackOff|ErrImagePull' || true)"
if [ -n "$pods" ]; then
  echo "FAIL: pods in $NAMESPACE cannot pull their image:" >&2
  echo "$pods" >&2
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "OK: $WORKLOAD runs the expected image"
