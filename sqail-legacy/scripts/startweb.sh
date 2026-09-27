#!/usr/bin/env bash
PORT=5173
pid=$(lsof -ti :"$PORT" 2>/dev/null)
if [ -n "$pid" ]; then
  echo "Killing existing process on port $PORT (PID: $pid)"
  kill "$pid" 2>/dev/null
  sleep 0.5
fi
cd "$(dirname "$0")/../sqail.portal" && pnpm dev
