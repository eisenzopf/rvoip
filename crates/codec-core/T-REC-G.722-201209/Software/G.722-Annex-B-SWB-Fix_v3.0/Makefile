#
# Makefile for GNU gcc/g++ compiler
#
.SUFFIXES: .o .c .cpp .h

# Directory locations
SRC_DIR   = src/mainlib
MATH_DIR  = src/basic_math
BASOP_DIR = src/basicOp_stl2009_v2.3
TEST_DIR  = src/test_codec
OUT_DIR=out

SUBDIRS   = $(SRC_DIR) $(MATH_DIR) $(BASOP_DIR) $(FLC_DIR) $(TEST_DIR)

MAKE      =  make
CC        =  gcc
CXX       =  g++

all: $(SUBDIRS)

$(SUBDIRS)::
	$(MAKE) -C $@ "CC=$(CC)" "CXX=$(CXX)"

clean: 
	$(foreach DIR, $(SUBDIRS), $(MAKE) -C $(DIR) clean;)
	/bin/rm -f $(TEST_DIR)/*.exe
	/bin/rm -rf $(OUT_DIR)

test:
	./test.sh test

proc:
	./test.sh proc

comp:
	./test.sh comp
