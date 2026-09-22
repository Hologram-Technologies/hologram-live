#!/usr/bin/env bash
# docker login against a registry behind auth.htpasswd (plan P6 T1): the
# anonymous push is refused with the Basic challenge, a bad password is
# refused, a good login pushes and pulls with one digest, and a logout ends it.
# The registry under test listens on $LOGIN (default 127.0.0.1:5003) with the
# gate's password file: gate / gate-password.
source "$(dirname "$0")/lib.sh"
LOGIN=${LOGIN:-127.0.0.1:5003}

challenge=$(curl -s -o /dev/null -w '%{http_code} %header{www-authenticate}' "http://$LOGIN/v2/")
same "$challenge" '401 Basic realm="Registry Realm"' "the challenge on /v2/"

dir=$(mktemp -d)
printf 'gate c login\n' > "$dir/hello.txt"
printf 'FROM scratch\nCOPY hello.txt /hello.txt\n' > "$dir/Dockerfile"
tag="$LOGIN/gate-c/login:v1"
docker build -q -t "$tag" "$dir" > /dev/null

docker logout "$LOGIN" > /dev/null 2>&1 || true
if docker push -q "$tag" > /dev/null 2>&1; then fail "an anonymous push went through"; fi
if printf 'wrong' | docker login -u gate --password-stdin "$LOGIN" > /dev/null 2>&1; then fail "a bad password logged in"; fi

printf 'gate-password' | docker login -u gate --password-stdin "$LOGIN" > /dev/null
docker push -q "$tag" > /dev/null
pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")
docker image rm "$tag" > /dev/null
docker pull -q "$tag" > /dev/null
same "$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")" "$pushed" "the digest pushed and pulled behind the login"

docker logout "$LOGIN" > /dev/null
docker image rm "$tag" > /dev/null
if docker pull -q "$tag" > /dev/null 2>&1; then fail "a pull after logout went through"; fi
printf 'login: challenge, refusals, push and pull behind the login, logout: ok (%s)\n' "$pushed"
