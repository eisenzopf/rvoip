/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
*------------------------------------------------------------------------
*  File: softbit.c
*  Function: Conversion between hardbit and softbit
*------------------------------------------------------------------------
*/

#include "errexit.h"
#include "softbit.h"

#define  CHAR_BIT  8

/*----------------------------------------------------------------
Function:
Converts hardbit to G.192 softbit.
Return value
None
----------------------------------------------------------------*/
void  hardbit2softbit(
  int                  nByte,
  const unsigned char* from,
  unsigned short*      to
)
{
  int i, j;
  unsigned char Btmp;
  unsigned short* Wtmp = to;

  for (i=0; i<nByte; i++)
  {
    Btmp = *(from+i);
    for (j=0; j<CHAR_BIT; j++)
    {
      if ( ( Btmp >> j ) & 0x1 )
      {
        *Wtmp = G192_BITONE;  /* 0x0081 means 1 */
      }
      else
      {
        *Wtmp = G192_BITZERO; /* 0x007f means 0 */
      }
      Wtmp++;
    }
  }
}

/*----------------------------------------------------------------
Function:
Converts G.192 softbit to hardbit.
Return value
None
----------------------------------------------------------------*/
void  softbit2hardbit(
  int                   nByte,
  const unsigned short* from,
  unsigned char*        to
)
{
  int i, j;
  unsigned char Btmp;
  const unsigned short* Wtmp = from;

  for (i=0; i<nByte; i++)
  {
    Btmp = 0;
    for (j=0; j<CHAR_BIT; j++)
    {
      if ( *Wtmp == G192_BITONE )
      {
        Btmp += ( 0x1 << j );
      }
      Wtmp++;
    }
    *(to+i) = Btmp;
  }
}

/*----------------------------------------------------------------
Function:
Checks G.192 format data.
Return value
Payload size, if the data is OK.
0, if frame erasure is detected.
----------------------------------------------------------------*/
int  checksoftbit(
  const unsigned short* bitstream
)
{
  int            i;
  unsigned short payloadsize = 0;

  if ( bitstream[idxG192_SyncHeader] == G192_SYNCHEADER )
  {
    payloadsize = bitstream[idxG192_BitstreamLength];
    bitstream += G192_HeaderSize;
    for (i=0; i<payloadsize; i++)
    {
      if ( *bitstream != G192_BITONE && *bitstream != G192_BITZERO )
        error_exit( "G192 format (bit) error." );
    }
  }
  else if ( bitstream[idxG192_SyncHeader] != G192_SYNCHEADER_FER )
  {
    error_exit( "G192 format (header) error." );
  }
  return (int)payloadsize;
}
