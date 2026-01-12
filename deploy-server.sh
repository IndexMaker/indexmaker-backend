#!/bin/bash

set -e

echo "🚀 Starting IndexMaker Deployment on Server..."

# Configuration
DEPLOY_DIR=/home/jafar/indexmaker-deploy
DATA_DIR=/home/jafar/indexmaker-deploy

cd $DEPLOY_DIR

# Check if required files exist
echo "📋 Checking required files..."
required_files=("indexmaker-backend-amd64-coingecko-env-var.tar" "docker-compose.yml" ".env" "indexmaker_dump.sql.gz")
for file in "${required_files[@]}"; do
    if [ ! -f "$file" ]; then
        echo "❌ Missing file: $file"
        exit 1
    fi
    echo "✅ Found: $file"
done

# Step 1: Load Docker image
echo ""
echo "📦 Loading Docker image..."
docker load -i indexmaker-backend-amd64.tar
echo "✅ Docker image loaded"

# Step 2: Create data directories
echo ""
echo "📁 Creating data directories..."
mkdir -p $DATA_DIR/postgres $DATA_DIR/backups
chmod 700 $DATA_DIR/postgres
echo "✅ Data directories created"

# Step 3: Pull PostgreSQL image
echo ""
echo "🐘 Pulling PostgreSQL image..."
docker-compose pull postgres
echo "✅ PostgreSQL image ready"

# Step 4: Start PostgreSQL
echo ""
echo "🚀 Starting PostgreSQL..."
docker-compose up -d postgres

# Wait for PostgreSQL to be ready
echo "⏳ Waiting for PostgreSQL to be ready..."
sleep 5
max_attempts=30
attempt=0
until docker exec indexmaker-postgres pg_isready -U postgres 2>/dev/null; do
  attempt=$((attempt + 1))
  if [ $attempt -eq $max_attempts ]; then
    echo "❌ PostgreSQL failed to start"
    docker-compose logs postgres
    exit 1
  fi
  echo "  Attempt $attempt/$max_attempts..."
  sleep 2
done
echo "✅ PostgreSQL is ready!"

# Step 5: Restore database
echo ""
echo "📥 Restoring database from dump..."
gunzip -c indexmaker_dump.sql.gz | docker exec -i indexmaker-postgres psql -U postgres -d indexmaker_db
echo "✅ Database restored!"

# Step 6: Verify database
echo ""
echo "🔍 Verifying database..."
docker exec indexmaker-postgres psql -U postgres -d indexmaker_db -c "SELECT COUNT(*) as table_count FROM information_schema.tables WHERE table_schema = 'public';"

# Step 7: Start backend
echo ""
echo "🚀 Starting backend..."
docker-compose up -d backend

# Wait for backend to be ready
echo "⏳ Waiting for backend to be ready..."
sleep 10
max_attempts=30
attempt=0
until curl -f http://localhost:3002/ &> /dev/null; do
  attempt=$((attempt + 1))
  if [ $attempt -eq $max_attempts ]; then
    echo "❌ Backend failed to start"
    docker-compose logs backend
    exit 1
  fi
  echo "  Attempt $attempt/$max_attempts..."
  sleep 2
done

# Step 8: Final verification
echo ""
echo "✅ Deployment Complete!"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🎉 IndexMaker Backend is Now Running!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Health check
echo "🏥 Health Check:"
curl -s http://localhost:3002/health
echo ""

# Test endpoint
echo ""
echo "🧪 Testing /indexes endpoint:"
curl -s http://localhost:3002/indexes | jq -r 'if . == [] then "No indexes found (database might still be syncing)" else "Found \(length) indexes" end'

# Show database stats
echo ""
echo "📊 Database Statistics:"
docker exec indexmaker-postgres psql -U postgres -d indexmaker_db -c "
SELECT 
  schemaname,
  tablename,
  pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) AS size,
  (SELECT COUNT(*) FROM pg_class WHERE relname = tablename) as exists
FROM pg_tables 
WHERE schemaname = 'public' 
ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC
LIMIT 10;
"

# Show running containers
echo ""
echo "🐳 Running Containers:"
docker-compose ps

# Show disk usage
echo ""
echo "💾 Disk Usage:"
df -h ~/ | grep -E 'Filesystem|/$'
du -sh ~/indexmaker-data/*

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📝 Useful Commands:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "View logs:           docker-compose logs -f"
echo "View backend logs:   docker-compose logs -f backend"
echo "Restart backend:     docker-compose restart backend"
echo "Stop services:       docker-compose down"
echo "Check health:        curl http://localhost:3002/health"
echo "Test endpoint:       curl http://localhost:3002/indexes | jq"
echo "Database shell:      docker exec -it indexmaker-postgres psql -U postgres -d indexmaker_db"
echo "Backup database:     docker exec indexmaker-postgres pg_dump -U postgres indexmaker_db | gzip > backup-\$(date +%Y%m%d).sql.gz"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
