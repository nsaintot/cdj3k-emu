#!/bin/sh
# Mark that the loader ran on this boot.
echo "e2e-hello: loader ran"
touch /tmp/e2e-hello.ran
