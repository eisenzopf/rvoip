CC=gcc
CFLAGS=-W -Wall -ansi -pedantic
TARGET=c_test
SRC=c_test.c

all: $(TARGET)

$(TARGET): $(SRC)
	$(CC) $(CFLAGS) -o $(TARGET) $(SRC)

clean:
	rm -f $(TARGET)
