Test a rebase that doesn't create a commit:

  $ enable amend rebase
  $ setconfig rebase.singletransaction=True
  $ setconfig experimental.copytrace=off
  $ setconfig rebase.experimental.inmemory=1
  $ setconfig rebase.experimental.inmemory.nomergedriver=False
  $ setconfig rebase.experimental.inmemorywarning="rebasing in-memory!"
  $ newrepo

Create a commit with a move + content change:
  $ newrepo
  $ echo "original content" > file
  $ hg add -q
  $ hg commit -q -m "base"
  $ echo "new content" > file
  $ hg mv file file_new
  $ hg commit -m "a"
  $ hg book -r . a

Recreate the same commit:
  $ hg up -q .~1
  $ echo "new content" > file
  $ hg mv file file_new
  $ hg commit -m "b"
  $ hg book -r . b
  $ cp -R . ../without_imm

Rebase one version onto the other, confirm it gets rebased out:
  $ hg rebase -r b -d a
  rebasing in-memory!
  rebasing 2:811ec875201f "b" (b tip)
  note: rebase of 2:811ec875201f created no changes to commit
  saved backup bundle to $TESTTMP/repo2/.hg/strip-backup/811ec875201f-889e3ef7-rebase.hg

Without IMM, this behavior is semi-broken: the commit is not rebased out and the
created commit is empty. (D8676355)
  $ cd ../without_imm
  $ setconfig rebase.experimental.inmemory=0
  $ hg rebase -r b -d a
  rebasing 2:811ec875201f "b" (b tip)
  warning: can't find ancestor for 'file_new' copied from 'file'!
  saved backup bundle to $TESTTMP/without_imm/.hg/strip-backup/811ec875201f-889e3ef7-rebase.hg
  $ hg export tip
  # HG changeset patch
  # User test
  # Date 0 0
  #      Thu Jan 01 00:00:00 1970 +0000
  # Node ID 7552e6b0bc4ab4ac16175ced4f08a54c31faf706
  # Parent  24483d5afe6cb1a13b3642b4d8622e91f4d1bec1
  b
  
