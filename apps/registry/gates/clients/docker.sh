#!/usr/bin/env bash
# docker build, push, pull and a push that shares a layer across repositories.
source "$(dirname "$0")/lib.sh"

dir=$(mktemp -d)
printf 'gate c\n' > "$dir/hello.txt"
printf 'FROM scratch\nCOPY hello.txt /hello.txt\n' > "$dir/Dockerfile"
docker build -q -t "$US/gate-c/docker:v1" "$dir" > /dev/null

docker push -q "$US/gate-c/docker:v1"
pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$US/gate-c/docker:v1")
docker image rm "$US/gate-c/docker:v1" > /dev/null
docker pull -q "$US/gate-c/docker:v1" > /dev/null
pulled=$(docker inspect --format '{{index .RepoDigests 0}}' "$US/gate-c/docker:v1")
same "$pushed" "$pulled" "the digest docker pushed and the digest it pulled"

# The same layers under a second repository: docker asks for a mount.
docker tag "$US/gate-c/docker:v1" "$US/gate-c/docker-copy:v1"
docker push -q "$US/gate-c/docker-copy:v1"

curl -fsS "http://$US/v2/gate-c/docker/tags/list" | grep -q '"v1"' || fail "tags/list does not show v1"
curl -fsS "http://$US/v2/_catalog" | grep -q 'gate-c/docker-copy' || fail "the catalogue does not show the second repository"
printf 'docker: push, pull, shared layer, listing: ok (%s)\n' "$pushed"
