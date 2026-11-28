#!/usr/bin/env bash
# Makes the D12 dump corpus (replica/databases/restore-copy.md §7): real dumps from the real tools, in Docker, of a
# small seeded `shop` database. The files are committed; run this again only to refresh them. Needs Docker, gzip, zstd,
# xz and bzip2. Hand-written hostile files live in hostile/ and are not made here.
set -euo pipefail
cd "$(dirname "$0")"
OUT="$PWD"
NET=dalan-dumps
docker network create "$NET" >/dev/null 2>&1 || true
cleanup() { docker rm -f dalan-dump-pg13 dalan-dump-pg16 dalan-dump-pg17 dalan-dump-my dalan-dump-maria dalan-dump-mongo >/dev/null 2>&1 || true; docker network rm "$NET" >/dev/null 2>&1 || true; }
trap cleanup EXIT

PG_SEED="
create role app_owner;
create type mood as enum ('happy', 'sad');
create table customers (id serial primary key, name text not null, email text unique, mood mood, created timestamptz default now());
create table orders (id bigint generated always as identity primary key, customer_id int references customers(id), total numeric(10,2), note text);
create view big_orders as select * from orders where total > 100;
create function order_count(c int) returns bigint language sql as \$\$ select count(*) from orders where customer_id = c \$\$;
create function touch() returns trigger language plpgsql as \$\$ begin new.note := coalesce(new.note, 'n/a'); return new; end \$\$;
create trigger orders_touch before insert on orders for each row execute function touch();
insert into customers (name, email, mood) values ('Ada', 'ada@example.test', 'happy'), ('Bo', 'bo@example.test', 'sad'), ('Cy', null, null);
insert into orders (customer_id, total, note) values (1, 120.50, 'tab\there'), (1, 9.99, E'line\nbreak'), (2, 300, 'back\\\\slash'), (3, 1, null);
alter table customers owner to app_owner;
grant select on orders to app_owner;
"

pg() { # version image
  local v=$1 img=$2 c="dalan-dump-pg$1"
  docker run -d --rm --name "$c" --network "$NET" -e POSTGRES_PASSWORD=pw -e POSTGRES_DB=shop "$img" >/dev/null
  until docker exec "$c" pg_isready -U postgres -d shop >/dev/null 2>&1; do sleep 1; done
  sleep 2
  docker exec -i "$c" psql -q -U postgres -d shop -v ON_ERROR_STOP=1 <<<"$PG_SEED"
  docker exec "$c" pg_dump -U postgres -Fc shop >"$OUT/pg$v.custom"
  docker exec "$c" pg_dump -U postgres -Ft shop >"$OUT/pg$v.tar"
  docker exec "$c" pg_dump -U postgres -Fp shop >"$OUT/pg$v.sql"
  docker exec "$c" pg_dump -U postgres -Fp --inserts shop >"$OUT/pg$v-inserts.sql"
  docker exec "$c" sh -c 'rm -rf /tmp/d && pg_dump -U postgres -Fd -f /tmp/d shop'
  rm -rf "$OUT/pg$v.dir" && docker cp "$c:/tmp/d" "$OUT/pg$v.dir"
  docker exec "$c" pg_dumpall -U postgres >"$OUT/pg$v-all.sql"
  docker rm -f "$c" >/dev/null
}
pg 13 postgres:13-alpine
pg 16 postgres:16
pg 17 postgres:17-alpine

MY_SEED="
create table customers (id int auto_increment primary key, name varchar(100) not null, email varchar(200) unique, created datetime default current_timestamp);
create table orders (id bigint auto_increment primary key, customer_id int, total decimal(10,2), note text, foreign key (customer_id) references customers(id));
create view big_orders as select * from orders where total > 100;
create procedure order_count(in c int) select count(*) from orders where customer_id = c;
create trigger orders_touch before insert on orders for each row set new.note = coalesce(new.note, 'n/a');
insert into customers (name, email) values ('Ada', 'ada@example.test'), ('Bo', 'bo@example.test'), ('Cy', null);
insert into orders (customer_id, total, note) values (1, 120.50, 'semi;colon'), (1, 9.99, 'quote''s'), (2, 300, 'back\\\\slash'), (3, 1, null);
"
docker run -d --rm --name dalan-dump-my --network "$NET" -e MYSQL_ROOT_PASSWORD=pw -e MYSQL_DATABASE=shop mysql:8.4 >/dev/null
until docker exec dalan-dump-my mysql -uroot -ppw -e 'select 1' shop >/dev/null 2>&1; do sleep 2; done
docker exec -i dalan-dump-my mysql -uroot -ppw shop <<<"$MY_SEED" 2>/dev/null
docker exec dalan-dump-my mysqldump -uroot -ppw --single-transaction --routines --triggers --events shop 2>/dev/null >"$OUT/mysql84.sql"
docker exec dalan-dump-my mysqldump -uroot -ppw --single-transaction --databases shop 2>/dev/null >"$OUT/mysql84-databases.sql"
docker rm -f dalan-dump-my >/dev/null

docker run -d --rm --name dalan-dump-maria --network "$NET" -e MARIADB_ROOT_PASSWORD=pw -e MARIADB_DATABASE=shop mariadb:11 >/dev/null
until docker exec dalan-dump-maria mariadb -uroot -ppw -e 'select 1' shop >/dev/null 2>&1; do sleep 2; done
docker exec -i dalan-dump-maria mariadb -uroot -ppw shop <<<"$MY_SEED"
docker exec dalan-dump-maria mariadb-dump -uroot -ppw --single-transaction --routines --triggers shop >"$OUT/mariadb11.sql"
docker rm -f dalan-dump-maria >/dev/null

docker run -d --rm --name dalan-dump-mongo --network "$NET" mongo:7 >/dev/null
until docker exec dalan-dump-mongo mongosh --quiet --eval 'db.runCommand({ping:1}).ok' >/dev/null 2>&1; do sleep 1; done
docker exec dalan-dump-mongo mongosh --quiet shop --eval '
db.customers.insertMany([{_id: 1, name: "Ada", tags: ["a"], at: new Date(0)}, {_id: 2, name: "Bo", n: NumberLong(5)}, {_id: 3, name: "Cy", d: NumberDecimal("1.10")}]);
db.customers.createIndex({name: 1});
db.orders.insertMany([{customer: 1, total: 120.5}, {customer: 2, total: 300}]);
db.createView("big_orders", "orders", [{$match: {total: {$gt: 100}}}]);' >/dev/null
docker exec dalan-dump-mongo mongodump --quiet --db=shop --archive >"$OUT/mongo7.archive"
docker exec dalan-dump-mongo mongodump --quiet --db=shop --archive --gzip >"$OUT/mongo7.archive.gz"
docker exec dalan-dump-mongo sh -c 'rm -rf /tmp/m && mongodump --quiet --db=shop --out=/tmp/m'
rm -rf "$OUT/mongo7.dir" && docker cp dalan-dump-mongo:/tmp/m "$OUT/mongo7.dir"
docker rm -f dalan-dump-mongo >/dev/null

# Outer compression of the single-file dumps (the readers must see through every one).
gzip -kf -9 pg16.sql pg16.custom mysql84.sql
zstd -qf pg17.sql -o pg17.sql.zst
xz -kf mariadb11.sql
bzip2 -kf pg13.sql
(cd pg16.dir && tar cf ../pg16.dir.tar .)
echo "Done: $(ls | wc -l) files in $OUT"
