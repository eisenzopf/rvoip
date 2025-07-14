This folder contains the G.722 test vectors in binary format, 16-bit words, little-endian (Intel architecture).

See the ITU-T Software Tool Library Users' Manual for endianness considerations.

NOTE - the letter "b" is added to the begining of all file names, compared to the ASCII version, to denote that these are the binary version of the files and avoid accidental overwriting.

File name	CRC-32  	Size (bytes)
bt1c1.xmt	0C3BFCA7	 32832
bt1c2.xmt	2D604685	  1600
bt1d3.cod	7398964F	 32832
bt2r1.cod	D1DAA1D1	 32832
bt2r2.cod	344EA5D0	  1600
bt3h1.rc0	E9250851	 32832
bt3h2.rc0	5330AE2E	  1600
bt3h3.rc0	3731AD7F	 32832
bt3l1.rc1	ED1B3993	 32832
bt3l1.rc2	8E8C4E2B	 32832
bt3l1.rc3	B7AA5569	 32832
bt3l2.rc1	AF00F31F	  1600
bt3l2.rc2	9143E92C	  1600
bt3l2.rc3	AE855C07	  1600
bt3l3.rc1	A5374659	 32832
bt3l3.rc2	687B250A	 32832
bt3l3.rc3	3605736B	 32832

The following is an excerpt of "makefile.cl" from the ITU-T Software Tool Library module G.722 [1] (which is a reference implementation of the G.722 algorithm). The sequence below illustrates the comparisons that need to be done in order to successfuly check the implementation. Here, the compiled versions of tstcg722.c and tstdg722.c are used (part of the G.722 module in the STL)


# ------------------------------------
# Test codec with test sequences
# ------------------------------------

TSTCG722=tstcg722 # exercise G.722 encoder digital test sequences 
TSTDG722=tstdg722 # exercise G.722 decoder digital test sequences 

test-tv: tv-step1 tv-step2

tv-step1: tstcg722
#	Should run without any error indication. 
	$(TSTCG722) bin/bt1c1.xmt bin/bt2r1.cod 
	$(TSTCG722) bin/bt1c2.xmt bin/bt2r2.cod 

tv-step2: tstdg722
#	Should run without any error indication. 
#	Files .rc0 indicate the codec mode of operation
	$(TSTDG722) bin/bt2r1.cod bin/bt3l1.rc1 bin/bt3h1.rc0 
	$(TSTDG722) bin/bt2r1.cod bin/bt3l1.rc2 bin/bt3h1.rc0 
	$(TSTDG722) bin/bt2r1.cod bin/bt3l1.rc3 bin/bt3h1.rc0 
	$(TSTDG722) bin/bt2r2.cod bin/bt3l2.rc1 bin/bt3h2.rc0 
	$(TSTDG722) bin/bt2r2.cod bin/bt3l2.rc2 bin/bt3h2.rc0 
	$(TSTDG722) bin/bt2r2.cod bin/bt3l2.rc3 bin/bt3h2.rc0 
	$(TSTDG722) bin/bt1d3.cod bin/bt3l3.rc1 bin/bt3h3.rc0 
	$(TSTDG722) bin/bt1d3.cod bin/bt3l3.rc2 bin/bt3h3.rc0 
	$(TSTDG722) bin/bt1d3.cod bin/bt3l3.rc3 bin/bt3h3.rc0 

 Reference:
[1] ITU-T G.191 (2010), "Software tools for speech and audio coding standardization.