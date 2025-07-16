coderd 1 testfile.raw testmid.t1
decoderd 1 testmid.1 testout.t1
coderd 2 testfile.raw testmid.t2
decoderd 2 testmid.2 testout.t2
coderd 3 testfile.raw testmid.t3
decoderd 3 testmid.3 testout.t3
rem fc
fc/b testmid.t1 testmid.1
fc/b testout.t1 testout.1
fc/b testmid.t2 testmid.2
fc/b testout.t2 testout.2
fc/b testmid.t3 testmid.3
fc/b testout.t3 testout.3

