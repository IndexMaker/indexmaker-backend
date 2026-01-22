-- Sync on-chain ITPs to database
-- Run with: psql $DATABASE_URL -f scripts/sync_itps.sql

-- Insert ITPs from on-chain events (skip if already exists)
INSERT INTO itps (orbit_address, arbitrum_address, name, symbol, description, methodology, initial_price, current_price, total_supply, state, created_at, updated_at)
SELECT * FROM (VALUES
  -- Nonce 0: Test ITP V2
  ('0xc71b518779176868f47e52a8c6ae4ac4d7bac934', '0x9a16717530ce1d58c0e1437eb6033a12108be5a7', 'Test ITP V2', 'TITP2', 'Test ITP created via bridge', 'Equal weight methodology', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW()),
  -- Nonce 1: Test ITP V3
  ('0x315e52f1f43278b5524d90ed0facaadcc81e1301', '0x799ff8ab331caef01fd1b7a7cbee06d6629e4df4', 'Test ITP V3', 'TITP3', 'Bridge test ITP V3', 'Equal weight test', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW()),
  -- Nonce 2: 3 (Index tracking 1INCH, A)
  ('0xe6de877a9e12731429b6b8420a5f437ae1dd4ba9', '0x81fa7680a25bbfc419fa6adbe8d4a54cb69bc0e8', '3', '3', 'Index tracking 1INCH, A', 'Market cap weighted index', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW()),
  -- Nonce 3: 3 (Index tracking 1INCH, A)
  ('0xc5b9d4a2d0efb568c7193e8d136d8066a3dfe8c6', '0x9ae119d7758a541ef9fe180f6e4f47356aa1512b', '3', '3', 'Index tracking 1INCH, A', 'Market cap weighted index', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW()),
  -- Nonce 4: 3 (Index tracking 1INCH, A)
  ('0xc36489e58ff3b271bdc1b5b7f9e89e01c06de156', '0x254e531cb4b33c1e8db3af9ca5cba14d79f785bd', '3', '3', 'Index tracking 1INCH, A', 'Market cap weighted index', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW()),
  -- Nonce 5: top 3
  ('0x1cee504ee8d9ec1012cb678c448d5f4cda0aea55', '0x943c13fef5e987e734a0438d08dd0d08bd6bbf67', 'top 3', 'TOP3', 'Index tracking 1INCH, A8', 'Market cap weighted index', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW()),
  -- Nonce 6: TOP3A
  ('0x6267eba7c3d853e14c3e62de682acabc47021118', '0xfa83f8bc546786bf89d945244194fede47137818', 'TOP3A', 'TOP3A', 'Index tracking A47, AAVE', 'Market cap weighted index', 1000000::numeric, 1000000::numeric, 0::numeric, 1::smallint, NOW(), NOW())
) AS t(orbit_address, arbitrum_address, name, symbol, description, methodology, initial_price, current_price, total_supply, state, created_at, updated_at)
WHERE NOT EXISTS (
  SELECT 1 FROM itps WHERE itps.orbit_address = t.orbit_address
)
ON CONFLICT (orbit_address) DO NOTHING;

-- Show results
SELECT id, name, symbol, orbit_address, arbitrum_address, state FROM itps ORDER BY id;
