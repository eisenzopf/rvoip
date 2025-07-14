This folder contains the G.722 test vectors in binary format, 16-bit words, big-endian (PowerPC architecture).

See the ITU-T Software Tool Library Users' Manual for endianness considerations.

NOTE - the letter "b" is added to the begining of all file names, compared to the ASCII version, to denote that these are the binary version of the files and avoid accidental overwriting.

File name	CRC-32  	Size (bytes)
bt1c1.xmt	015ACCE4	 32832
bt1c2.xmt	EAFC99B4	  1600
bt1d3.cod	1C85BE45	 32832
bt2r1.cod	0B904231	 32832
bt2r2.cod	F928980D	  1600
bt3h1.rc0	0BDE9C9C	 32832
bt3h2.rc0	3A54C7DF	  1600
bt3h3.rc0	8E8DEE65	 32832
bt3l1.rc1	90DD2D72	 32832
bt3l1.rc2	FE7C4611	 32832
bt3l1.rc3	20B5FFC4	 32832
bt3l2.rc1	D2599DE8	  1600
bt3l2.rc2	84041F43	  1600
bt3l2.rc3	F32628D9	  1600
bt3l3.rc1	8C12ED04	 32832
bt3l3.rc2	550534A7	 32832
bt3l3.rc3	9354E9CF	 32832

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