#!/bin/bash
# Concurrent Load Test for rust-agent

URL="http://localhost:8080/api/tasks"

prompts=(
  "Check system uptime"
  "Show current disk usage"
  "List running Docker containers"
  "Check system memory status"
  "Show active network interfaces"
  "Check CPU load average"
  "Display last 10 system log lines"
  "Check free storage space on root partition"
  "Show active system services"
  "List logged-in users"
)

echo "Starting concurrent dispatch of 10 tasks..."
start_time=$(date +%s)

for i in "${!prompts[@]}"; do
  (
    echo "Sending Task $((i+1)): ${prompts[$i]}"
    curl -s -X POST "$URL" \
      -H "Content-Type: application/json" \
      -d "{\"prompt\": \"${prompts[$i]}\"}" \
      > /dev/null
    echo "--> Task $((i+1)) returned."
  ) &
done

wait
end_time=$(date +%s)
echo "------------------------------------------------"
echo "All 10 tasks completed in $((end_time - start_time)) seconds!"
