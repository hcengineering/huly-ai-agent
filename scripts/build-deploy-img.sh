#!/usr/bin/env bash

for i in "$@"; do
  case $i in
    -h=*|--host=*)
      HOST="${i#*=}"
      shift
      ;;
    -p=*|--password=*)
      PASSWORD="${i#*=}"
      shift
      ;;
    -*|--*)
      echo "Unknown option $i"
      exit 1
      ;;
    *)
      ;;
  esac
done

echo "HOST  = ${HOST}"

sudo nerdctl build -t huly-ai-agent -f "./Dockerfile" .
sudo nerdctl save -o huly-ai-agent-image.tar huly-ai-agent
scp huly-ai-agent-image.tar ${HOST}:/home/user/huly-ai-agent-image.tar
echo 123qwe | ssh -tt ${HOST} "sudo ctr i import huly-ai-agent-image.tar"
