cp /usr/local/bin/spin .
docker build -f Dockerfile -t dtomcat .
rm ./spin
