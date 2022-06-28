#!/bin/bash
while IFS= read -r line; do
  	logger "$(date): PID of this script: $$"
	echo "$(date): $line from $$"
	sleep 3
#	exit 1
done
