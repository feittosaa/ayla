import os
from fastapi import Header, HTTPException

APP_TOKEN = os.getenv("AYLA_TOKEN")

def verify_token(x_ayla_token: str = Header(None)):
    if not APP_TOKEN or x_ayla_token != APP_TOKEN:
        raise HTTPException(status_code=401, detail="Unauthorized")
