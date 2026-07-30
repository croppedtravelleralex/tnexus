UPDATE _sqlx_migrations
SET checksum = decode(
  'fdc29eaff52e914c99ced35977934ddb72ad993b99d487e61aaca0f7be6549f5de88678a461979ddbaade6730878ddd6',
  'hex'
)
WHERE version = 2;
