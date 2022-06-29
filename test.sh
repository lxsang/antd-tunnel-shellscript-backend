#!/bin/bash
while IFS= read -r line; do
  	logger "$(date): PID of this script: $$, user $CUSER, ID $CID"
	echo "$(date): $line from $$, user $CUSER, ID $CID"
	sleep 3
#	exit 1
done
