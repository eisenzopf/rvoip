/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                   */
/* Version:       1.0 (based on G.722 v3.0 beta of the STL)   */
/* Revision Date: Nov.02, 2006                                */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/


/* Copyright and version information from the original G.722 header:
============================================================================

DECG722.C 
~~~~~~~~~

Original author:
~~~~~~~~~~~~~~~~
J-P PETIT 
CNET - Centre Lannion A
LAA-TSS                         Tel: +33-96-05-39-41
Route de Tregastel - BP 40      Fax: +33-96-05-13-16
F-22301 Lannion CEDEX           Email: petitjp@lannion.cnet.fr
FRANCE

History:
~~~~~~~~
14.Mar.95  v1.0       Released for use ITU-T UGST software package Tool
                      based on the CNET's 07/01/90 version 2.00
01.Jul.95  v2.0       Changed function declarations to work with many compilers;
                      reformated <simao@ctd.comsat.com>
22.May.06  v2.3       Updated with g192 format reading and basic index domain PLC functionality. 
                      <{nicklas.sandgren,jonas.svedberg}@ericsson.com>
23.Aug.06  v3.0 beta  Updated with STL2005 v2.2 basic operators and G.729.1 methodology
                      <{balazs.kovesi,stephane.ragot}@orange-ft.com>
============================================================================
*/

/* Standard prototypes */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef VMS
#include <stat.h>
#else
#include <sys/stat.h>
#endif

/* G.722- and UGST-specific prototypes */
#include "g722.h"
#include "ugstdemo.h"
#include "g722_com.h"

#include "stl.h"
#include "g722plc.h"
#if DMEM
#include "memutil.h"
#endif

/* local prototypes */
void display_usage ARGS((void));

short g192_to_byte(short nb_bits, short *code_byte, short *inp_bits, short *N_samples);
void set_index(short value, short *code,short n_bytes);

short g192_to_byte(short nb_bits, short *code_byte, short *inp_bits, short *N_samples){
   long i,j,k;
   short bit,mask,mode=-1;;

   /* convert soft bits value [-127 ... +127] range to hard bits (G192_ONE or G192_ZERO) */
   for(i=0;i < nb_bits;i++){      
      if(inp_bits[i]&0x0080){ /* look at sign bit only */
         inp_bits[i] = G192_ONE;
      } else {
         inp_bits[i] = G192_ZERO;
      }
   } 

   /* for frame sizes of 30 ms or less the mode and sample frame sizes can be uniquely
      decoded from the frame size in bits from the G.192 format */
   if(nb_bits == 480){
      mode       = 3;    // 48 kb/s
      if(*N_samples != -1 && *N_samples != 160)
         fprintf(stderr,"Specified frame size (%d) overridden with G.192 derived frame size (160)",*N_samples);
      *N_samples = 160;  // 10 ms
   }
   else if(nb_bits == 560){
      mode       = 2;    // 56 kb/s
      if(*N_samples != -1 && *N_samples != 160)
         fprintf(stderr,"Specified frame size (%d) overridden with G.192 derived frame size (160)",*N_samples);
      *N_samples = 160;  // 10 ms
   }
   else if(nb_bits == 640){
      mode       = 1;    // 64 kb/s
      if(*N_samples != -1 && *N_samples != 160)
         fprintf(stderr,"Specified frame size (%d) overridden with G.192 derived frame size (160)",*N_samples);
      *N_samples = 160;  // 10 ms
   }
   else if(nb_bits == 2*480){
      mode       = 3;    // 48 kb/s
      if(*N_samples != -1 && *N_samples != 320)
         fprintf(stderr,"Specified frame size (%d) overridden with G.192 derived frame size (320)",*N_samples);
      *N_samples = 320;  // 20 ms
   }
   else if(nb_bits == 2*560){
      mode       = 2;    // 56 kb/s
      if(*N_samples != -1 && *N_samples != 320)
         fprintf(stderr,"Specified frame size (%d) overridden with G.192 derived frame size (320)",*N_samples);
      *N_samples = 320;  // 20 ms
   }
   else if(nb_bits == 2*640){
      mode       = 1;    // 64 kb/s
      if(*N_samples != -1 && *N_samples != 320)
         fprintf(stderr,"Specified frame size (%d) overridden with G.192 derived frame size (320)",*N_samples);
      *N_samples = 320;  // 20 ms
   }
   else{
      /* for frame sizes greater than 20 ms the decoder needs the command line frame size
         specification in order to uniquely determine the mode */
      if(*N_samples == -1){
         fprintf(stderr,"FATAL ERROR: For frame sizes greater than 20 ms or Nbits=0 in the G.192\n");
         fprintf(stderr,"             header, the deocder cannot uniquely determine the mode\n");
         fprintf(stderr,"             and frame size without the frame size being specified at\n");
         fprintf(stderr,"             the command line with -fsize\n");
         exit(0);
      }

      if(nb_bits==(*N_samples/2)*8)
         mode=1;
      else if(nb_bits==(*N_samples/2)*7)
         mode=2;
      else if(nb_bits==(*N_samples/2)*6)
         mode=3;

      if(*N_samples != 160*(*N_samples/160)){
         fprintf(stderr,"FATAL ERROR: Apparent invalid frame size (%d samples)\n",*N_samples);
         fprintf(stderr,"             Must be mutiple of 10 ms (160 samples\n");
         exit(0);
      }
   }

   set_index(0,code_byte,(short)(*N_samples/2));

   if(nb_bits==0){
      mode = -1; /* flag for NO_DATA or BAD SPEECH frame, without payload */
   } else {
      /* special ordering for scalability reasons */
      /* write [ b2*n, b3*n, b4*n, b5*n, b6*n, b7*n, b1*n, b0*n]  to enable truncation of G.722 g192 frames */ 
      /*  "core"/non-scalable layer is read first b2s-b3s-b4s-b5s */
      /* b6s,b7s are semi scalable, (b6,b7) scalability is not part of the g.722 standard */
      j=0;
      for (bit=2;bit<8;bit++){
         mask=(0x0001<<(bit));
         for(i=0; i < (*N_samples/2); i++,j++){
            if(inp_bits[j] == G192_ONE){
               code_byte[i] |= mask; /* set bit */  
            } 
            /* leave bit zero */
         }
      }

      /* embedded layers last in G.192 frame */
      /* read b1s followed by b0s if available */
      k=0;                /* 64 kbps */
      if(mode==2) {k=1;}; /* 56 kbps */
      if(mode==3) {k=2;}; /* 48 kbps*/
      for (bit=1;bit>=k;bit--){
         mask=(0x0001<<(bit)); 
         for(i=0; i < (*N_samples/2); i++,j++){
            if(inp_bits[j] == G192_ONE){
               code_byte[i] |= mask;
            } 
         }
      }
   }
   return mode;
}

void set_index(short value,short *code,short n_shorts){
   long i;
   FOR(i=0;i<n_shorts;i++){
      code[i]=value;
#ifdef WMOPS
    move16();
#endif
   }
}


#define P(x) printf x
void display_usage()
{
   /* Print Message */
   printf ("\n\n");
   printf ("\n***************************************************************");
   printf ("\n* PROCESSING THE DECODER OF ITU-T G.722 WIDEBAND SPEECH CODER *");
   printf ("\n* COPYRIGHT CNET LANNION A TSS/CMC Date 24/Aug/90             *");
   printf ("\n* COPYRIGHT Ericsson AB.           Date 22/May/06             *");
   printf ("\n* COPYRIGHT France Telecom R&D     Date 23/Aug/06             *");
   printf ("\n* COPYRIGHT Broadcom Corporation   Date  2/Nov/06             *");
   printf ("\n***************************************************************\n\n");

   /* Quit program */
   P(("USAGE: \n"));
   P(("  decg722 [-options] file.adp file.outp \n"));
   P(("or \n"));
   P(("  decg722 [-fsize N] file.g192 file.out \n\n"));

   exit(-128);
}
#undef P

/*
**************************************************************************
***                                                                    ***
***        Demo-Program for decoding G.722 frames with errors          ***
***                                                                    ***
**************************************************************************
*/
int main (argc, argv)
int argc; 
char *argv[];
{
   /* Local Declarations */
   short  mode; /* actual decoder synthesis mode*/

   g722_state      decoder;
   struct WB_PLC_State      plc_state;

   /* Encode and decode operation specification */
   /* Sample buffers */
   short  code[MAX_BYTESTREAM_BUFFER];   /* byte stream buffer , 1 byte(8 bit) at  8 kHz */
   short  outcode[MAX_OUTPUT_SP_BUFFER]; /* speech buffer */
   short  inp_frame[2+MAX_BITLEN_SIZE]; /* g192 inpFrame , 1 bit is stored per 16 bit Word*/

   long   frames=1; /* frame counter */
   short  bfi;

   /* File variables */
   char            FileIn[MAX_STR], FileOut[MAX_STR];
   FILE            *F_cod, *F_out;
   long            iter=0;
   short           N=-1, smpno=0;
   long            i=0;
   char            bs_format = g192;
   char            tmp_type;   /* for input type checking*/
   short           header[2];  /* g192 header size*/

#ifdef WMOPS
    short spe1Id = -1;
    short spe2Id = -1;
    double average1; 
    long num_frames1;
    double average2; 
    long num_frames2;
#endif

#ifdef VMS
   char            mrs[15];
#endif

   /* Progress flag indicator */
   static char     quiet=0, funny[9] = "|/-\\|/-\\";

   fprintf(stderr, "\n******************************************************************************\n");
   fprintf(stderr, "ITU-T G.722 Appendix III\n\n");
   fprintf(stderr, "Version: 1.0\n");
   fprintf(stderr, "Revision Date: Nov.02, 2006\n\n");
   fprintf(stderr, "This software has been developed by Broadcom Corporation. \n\n");
   fprintf(stderr, "Copyright (c)  Broadcom Corporation 2006.  All rights reserved. \n\n");
   fprintf(stderr, "COPYRIGHT : This file is the property of Broadcom Corporation.  It cannot \n");
   fprintf(stderr, "be copied, used, distributed or modified without obtaining authorization\n");
   fprintf(stderr, "from Broadcom Corporation.  If such authorization is provided, any modified\n");
   fprintf(stderr, "version of the software must contain this header.\n\n");
   fprintf(stderr, "WARRANTIES : This software is made available by  Broadcom Corporation in the \n");
   fprintf(stderr, "hope that it will be useful, but without any warranty, including but not \n");
   fprintf(stderr, "limited to any warranty of non-infringement of any third party intellectual \n");
   fprintf(stderr, "property rights.  Broadcom Corporation is not liable for any direct or \n");
   fprintf(stderr, "indirect consequence  or damages related to the use of the provided software, \n");
   fprintf(stderr, "whether or not foreseeable.\n");
   fprintf(stderr, "******************************************************************************\n");

   printf ("\n***************************************************************\n");
   printf ("* Original G.722 Copyright Header:                            *\n");
   printf ("* PROCESSING THE DECODER OF ITU-T G.722 WIDEBAND SPEECH CODER *\n");
   printf ("* COPYRIGHT CNET LANNION A TSS/CMC Date 24/Aug/90             *\n");
   printf ("* COPYRIGHT Ericsson AB.           Date 22/May/06             *\n");
   printf ("* COPYRIGHT France Telecom R&D     Date 23/Aug/06             *\n");
   printf ("***************************************************************\n\n");
   /* *** ......... PARAMETERS FOR PROCESSING ......... *** */
   /* GETTING OPTIONS */
   if (argc < 2)
      display_usage();
   else {
      while (argc > 1 && argv[1][0] == '-')
         if (strcmp(argv[1], "-fsize") == 0){
            /* Define Frame size for g192 operation and file reading */
            N = atoi(argv[2]);
            if(( N > MAX_OUTPUT_SP_BUFFER) || (N <=0) || (N&0x0001)) {
               fprintf(stderr, "ERROR! Invalid frame size \"%s\" in command line\n\n",argv[2]);
               display_usage();
            }
            /* Move argv over the option to the next argument */
            argv+=2;
            argc-=2;
         } else if (strcmp(argv[1], "-q") == 0) {
            /* Don't print progress indicator */
            quiet = 1;

            /* Move argv over the option to the next argument */
            argv++;
            argc--;
         } else if (strcmp(argv[1], "-h") == 0 || strcmp(argv[1], "-help") == 0){
            /* Print help */
            display_usage();
         } else {
            fprintf(stderr, "ERROR! Invalid option \"%s\" in command line\n\n",
               argv[1]);
            display_usage();
         }
   }

   /* Now get regular file parameters */
   GET_PAR_S(1, "_Input File: .................. ", FileIn);
   GET_PAR_S(2, "_Output File: ................. ", FileOut);

   /* Open input file */
   if ((F_cod = fopen (FileIn, RB)) == NULL){
      KILL(FileIn, -2);
   }
   /* Open output file */
   if ((F_out = fopen (FileOut, WB)) == NULL) {
      KILL(FileOut, -2);
   }

   /* Reset lower and upper band encoders */
   g722_reset_decoder(&decoder);
   Reset_WB_PLC(&plc_state);
#ifdef WMOPS
   spe1Id = getCounterId("lost frame processing");
   setCounter(spe1Id);
   Init_WMOPS_counter();
   spe2Id = getCounterId("received frame processing");
   setCounter(spe2Id);
   Init_WMOPS_counter();
#endif

   /* Read an analysis frame of bits from input bit stream file and decode */

   /* for g192 inputs, the synch header and frame size has to be read first */
   /* then the soft bits may be read if they are available */
   /* bits are stored b2s,b3s,b4s,b5s,b6s,b7s and b1s,b0s  to allow frame truncation*/

   /* Do preliminary inspection in the INPUT BITSTREAM FILE to check
      that it has a correct format (g192) */
      
   i = check_eid_format(F_cod, FileIn, &tmp_type);
   /* Check whether the  required input format matches with the one in the file */
   if (i != bs_format) {
      /* The input bitstream format is not g192 */
      fprintf (stderr, "*** Illegal input bitstream format: %s (should be %s) ***\n",
               format_str(i),format_str((int)bs_format));
      HARAKIRI("\nExiting...\n\n",1);
   }
      
   {  /* check input file for valid initial G.192 synchronism headers */
      short sync_header=0;
      fread(header, sizeof(short), 2, F_cod); /* Get presumed first G.192 sync header */
      i = header[1]; /* header[1] should have the frame length */
        
      /* advance file to the (presumed) next G.192 sync header */
      fseek(F_cod, (long)(header[1])*sizeof(short), SEEK_CUR);
      fread(header, sizeof(short), 2, F_cod);  /* get (presumed) next G.192 sync header */

      if ((header[0] & 0xFFF0) == (G192_FER & 0xFFF0)) { /* Verify */
         sync_header = 1;
      }    
      fseek(F_cod, 0l, SEEK_SET); /* Rewind BP file */
      if(sync_header==0){
         HARAKIRI("Error::Input bitstream MUST have valid G192 sync_headers \n\n",1);
      }
   }
      
   /* start actual g.192 frame loop */
   while (fread (header, sizeof (short), 2, F_cod) == 2) {
      if(header[1] > MAX_BITLEN_SIZE){
         fprintf(stderr,"FATAL ERROR: Frame size (%d) too large\n",header[1]);
         exit(0);
      }
      if(!((short)fread (inp_frame, sizeof (short), header[1], F_cod) == header[1])){
         HARAKIRI("Error::Could not read complete frame of input bitstream  \n\n",1);
         break;
      } 
      else {          /* normal decoding */
         if (!quiet){ /* progress character*/
            fprintf(stderr, "%c\r", funny[(iter/N/2) % 8]);
         } 
         mode = g192_to_byte(header[1],code,inp_frame,&N);

         if(header[0] != G192_SYNC || mode == -1){ /* bad frame, (with zero length or valid G.722 length) */ 
#ifdef WMOPS
            setFrameRate(16000, N);  // only provides correct average WMOPS for constant N
            setCounter(spe1Id);
            fwc();
            Reset_WMOPS_counter();
#endif
            bfi=1;
            smpno = G722DecWithPLC(code, outcode, mode, N, &decoder, &plc_state, bfi);
         } 
         else {  /* good frame, update index memory mem_code and mode memory mem_mode */
#ifdef WMOPS
            setFrameRate(16000, N);  // only provides correct average WMOPS for constant N
            setCounter(spe2Id);
            fwc();
            Reset_WMOPS_counter();
#endif
            bfi=0;

            smpno = G722DecWithPLC(code, outcode, mode, N, &decoder, &plc_state, bfi);
         }
      }
 
      /* Update sample counter */
      iter += smpno;
      /* Save a frame of decoded speech samples */
      if ((short)fwrite ((char *) outcode, sizeof (Word16), N, F_out) != N) {
         KILL(FileOut,-4);
      }
      frames++;
   } /* end while fread */
#ifdef WMOPS
   setCounter(spe1Id);
   fwc();
   WMOPS_output(0, &average1, &num_frames1);
   setCounter(spe2Id);
   fwc();
   WMOPS_output(0, &average2, &num_frames2);
   printf("Global average %.3f WMOPS\n", (average1 * num_frames1 + average2 * num_frames2)/(num_frames1 + num_frames2));
#endif

   /* Close input and output files */
   fclose(F_out); 
   fclose(F_cod);
#if DMEM
   DMEM_output();
#endif

   /* Exit with success for non-vms systems */
#ifndef VMS
   return (0);
#endif
}
